//! Rustload — fast, lightweight, self-hosted download manager.
//!
//! The `rustload` binary runs it as a server configured from the environment; the desktop app
//! embeds the same [`Server`].

mod api;
mod backup;
mod checksum;
mod config;
mod db;
mod debrid;
mod diagnostics;
mod engine;
mod error;
mod events;
mod extract;
mod hosters;
mod integrations;
mod logging;
mod media;
mod models;
mod privileges;
mod settings;
mod stats;
mod util;
mod web;

use std::{net::SocketAddr, path::Path, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use tokio::{sync::broadcast, task::JoinHandle};
use tokio_util::sync::{CancellationToken, WaitForCancellationFuture};
use tracing_appender::non_blocking::WorkerGuard;

use crate::{engine::Engine, events::EventBus, settings::SettingsStore};

pub use crate::{api::pairing::PairRequest, config::Config, events::Event};

/// Debrid accounts are re-checked this often (premium expiry, quota).
const ACCOUNT_CHECK_INTERVAL: Duration = Duration::from_secs(6 * 3600);

/// A running Rustload: the download engine, its integrations and the web interface.
pub struct Server {
    handle: Handle,
    shutdown: CancellationToken,
    serve_task: JoinHandle<Result<()>>,
    engine_task: JoinHandle<()>,
    _log_guard: WorkerGuard,
}

impl Server {
    /// Opens the database, resumes the queue and serves the web interface. Returns once the
    /// interface accepts connections.
    pub async fn start(config: Config) -> Result<Self> {
        let config = Arc::new(config);
        // Must happen before any file is created so everything belongs to PUID/PGID.
        let notes = privileges::apply(&config)?;
        let (logs, log_guard) = logging::init(&config.log_filter, &config.logs_dir())?;
        for note in notes {
            tracing::info!("{note}");
        }
        let _ = rustls::crypto::ring::default_provider().install_default();
        tracing::info!("Rustload {} starting", env!("CARGO_PKG_VERSION"));

        let listener = api::bind(config.bind).await?;
        let address = listener.local_addr().context("cannot read the listening address")?;
        std::fs::create_dir_all(&config.download_root)
            .with_context(|| format!("cannot create download folder {}", config.download_root.display()))?;
        let db = db::open(&config.database_path()).await?;
        db::categories::seed_defaults(&db).await?;
        api::bootstrap_admin(&db, config.admin_user.as_deref(), config.admin_password.as_deref()).await?;

        let settings = Arc::new(SettingsStore::load(db.clone()).await?);
        let events = Arc::new(EventBus::new());
        let shutdown = CancellationToken::new();
        let engine = Engine::new(db, config.clone(), settings, events.clone(), shutdown.clone()).await?;

        if !config.auth_enabled {
            tracing::warn!("authentication is disabled (RUSTLOAD_AUTH=false): protect access to this instance");
        }
        match engine.extractor.tool_name() {
            Some(tool) => tracing::info!("archive extraction with {tool}"),
            None if media::tools::can_install_seven_zip() => tracing::warn!(
                "7-Zip not found: only ZIP archives can be extracted; install it from Settings › Extraction"
            ),
            None => tracing::warn!("7-Zip not found: only ZIP archives can be extracted"),
        }

        {
            let (events, token) = (events.clone(), shutdown.clone());
            tokio::spawn(async move { events.run_flusher(token).await });
        }
        tokio::spawn(integrations::watch::run(engine.clone()));
        tokio::spawn(integrations::clicknload::supervise(engine.clone(), config.cnl_bind));
        tokio::spawn(integrations::telegram::supervise(engine.clone()));
        tokio::spawn(refresh_accounts(engine.clone()));
        tokio::spawn(engine::media::maintain(engine.clone()));
        let engine_task = tokio::spawn(engine.clone().run());

        let state = api::AppState::new(engine.clone(), config.clone(), logs, address);
        let serve_task = {
            let (state, token) = (state.clone(), shutdown.clone());
            tokio::spawn(async move {
                let served = api::serve(listener, state, token.clone()).await;
                // The engine stops with the interface, whatever stopped it.
                token.cancel();
                served
            })
        };
        let handle = Handle { engine, state };
        Ok(Self { handle, shutdown, serve_task, engine_task, _log_guard: log_guard })
    }

    /// What the desktop app uses to drive the server while it runs.
    pub fn handle(&self) -> Handle {
        self.handle.clone()
    }

    /// Resolves when the server stops: [`Server::stop`] was called or the web interface failed.
    pub fn stopping(&self) -> WaitForCancellationFuture<'_> {
        self.shutdown.cancelled()
    }

    /// Stops downloading cleanly (transfers keep what they downloaded) and closes the interface.
    pub async fn stop(self) -> Result<()> {
        tracing::info!("shutting down");
        self.shutdown.cancel();
        let served = self.serve_task.await.context("web server task panicked")?;
        self.engine_task.await.context("engine task panicked")?;
        tracing::info!("bye");
        served
    }
}

/// A cheap, shareable reference to a running [`Server`].
#[derive(Clone)]
pub struct Handle {
    engine: Arc<Engine>,
    state: api::AppState,
}

impl Handle {
    /// Where the web interface listens.
    pub fn address(&self) -> SocketAddr {
        self.state.address
    }

    /// Live events, the ones the interface receives: `stats` every second, `notice` for messages
    /// worth a notification, `queue`/`history` when they change.
    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.engine.events.subscribe()
    }

    pub fn is_paused(&self) -> bool {
        self.engine.is_paused()
    }

    /// Pauses or resumes the whole queue, like the interface's *Pause all*.
    pub async fn set_paused(&self, paused: bool) -> Result<()> {
        self.engine.set_queue_paused(paused).await
    }

    pub fn download_root(&self) -> &Path {
        &self.engine.config.download_root
    }

    /// Lets the browser extension pair with the desktop app: `approve` shows the request to the
    /// person and resolves to their answer (`POST /api/desktop/pair`, [`Config::desktop`] only).
    pub fn set_pairing_approver<F, Fut>(&self, approve: F)
    where
        F: Fn(PairRequest) -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = bool> + Send + 'static,
    {
        self.state.pairing.set_approver(Arc::new(move |request| Box::pin(approve(request))));
    }

    /// A single-use address that opens the interface already logged in, at `path` (`/` for the
    /// dashboard, `/?add=…` to add links). Only for [`Config::desktop`], whose interface answers on
    /// this computer only.
    pub fn login_url(&self, path: &str) -> Result<String> {
        anyhow::ensure!(self.engine.config.desktop, "login addresses are only issued to the desktop app");
        let code = self.state.auth.issue_handoff();
        let next = percent_encoding::utf8_percent_encode(path, percent_encoding::NON_ALPHANUMERIC);
        Ok(format!("http://127.0.0.1:{}/api/auth/handoff?code={code}&next={next}", self.address().port()))
    }
}

async fn refresh_accounts(engine: Arc<Engine>) {
    let mut delay = Duration::from_secs(10);
    loop {
        tokio::select! {
            _ = engine.shutdown_token().cancelled() => return,
            _ = tokio::time::sleep(delay) => {}
        }
        engine.debrid.check_all().await;
        delay = ACCOUNT_CHECK_INTERVAL;
    }
}
