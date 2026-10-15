//! Rustload — fast, lightweight, self-hosted download manager.

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

use std::{process::ExitCode, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use tokio_util::sync::CancellationToken;

use crate::{config::Config, engine::Engine, events::EventBus, settings::SettingsStore};

/// Debrid accounts are re-checked this often (premium expiry, quota).
const ACCOUNT_CHECK_INTERVAL: Duration = Duration::from_secs(6 * 3600);

#[tokio::main]
async fn main() -> ExitCode {
    match run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("rustload: {error:#}");
            ExitCode::FAILURE
        }
    }
}

async fn run() -> Result<()> {
    let config = Arc::new(Config::from_env()?);
    // Must happen before any file is created so everything belongs to PUID/PGID.
    let notes = privileges::apply(&config)?;
    let (logs, _log_guard) = logging::init(&config.log_filter, &config.logs_dir())?;
    for note in notes {
        tracing::info!("{note}");
    }
    let _ = rustls::crypto::ring::default_provider().install_default();
    tracing::info!("Rustload {} starting", env!("CARGO_PKG_VERSION"));

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
    {
        let token = shutdown.clone();
        tokio::spawn(async move {
            wait_for_signal().await;
            tracing::info!("shutting down");
            token.cancel();
        });
    }

    let served = api::serve(api::AppState::new(engine, config.clone(), logs), config.bind, shutdown.clone()).await;
    shutdown.cancel();
    engine_task.await.context("engine task panicked")?;
    tracing::info!("bye");
    served
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

async fn wait_for_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => std::future::pending::<()>().await,
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();
    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
