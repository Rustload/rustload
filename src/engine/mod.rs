//! The download engine: owns the queue, starts work when slots are free, and exposes the
//! commands used by the API (pause, resume, retry, delete...).

mod checker;
mod disk_guard;
pub mod grabber;
mod handler;
pub mod intake;
mod lifecycle;
pub mod limiter;
pub mod live;
mod m3u;
mod magnet;
pub mod media;
pub mod naming;
mod postprocess;
mod remote;
mod resolve;
mod speed_rules;
pub mod storage;
mod task;
pub mod transfer;
#[cfg(test)]
mod transfer_tests;
mod verify;

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc, Mutex, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::sync::Notify;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

use crate::{
    config::Config,
    db::{self, Db, downloads::DownloadUpdate, packages::PackageUpdate},
    debrid::DebridManager,
    events::EventBus,
    extract::Extractor,
    hosters::Hosters,
    integrations::notify::Notifier,
    models::{DownloadRow, DownloadStatus, PackageRow, PackageStatus, Stage},
    settings::{Settings, SettingsStore},
    stats::{Snapshot, StatsRecorder},
};
use limiter::RateLimiter;
use live::{LiveRegistry, StopReason};
pub use remote::torrent_path as remote_torrent_path;

const QUEUE_PAUSED_KEY: &str = "queue_paused";
/// How long a learned per-host connection limit is trusted.
const CONNECTION_LIMIT_TTL: Duration = Duration::from_secs(3600);
const USER_AGENT: &str = concat!("Rustload/", env!("CARGO_PKG_VERSION"), " (+https://github.com/rustload/rustload)");

/// Remote (debrid) processing state of a magnet/torrent package, shown in the UI.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
pub struct DebridProgress {
    pub status: String,
    pub progress: f64,
    pub speed: u64,
    pub seeders: Option<u32>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct DiskInfo {
    pub free: u64,
    pub total: u64,
}

pub struct Engine {
    pub db: Db,
    pub config: Arc<Config>,
    pub settings: Arc<SettingsStore>,
    pub events: Arc<EventBus>,
    pub debrid: Arc<DebridManager>,
    pub live: LiveRegistry,
    pub limiter: RateLimiter,
    pub extractor: Extractor,
    pub hosters: Hosters,
    pub notifier: Notifier,
    pub stats: StatsRecorder,
    /// yt-dlp and ffmpeg (media sites).
    pub media: crate::media::MediaTools,
    pub started_at: Instant,
    /// Client for provider APIs and notifications.
    pub api_http: reqwest::Client,
    /// Client for file transfers, rebuilt when timeouts or user agent change.
    download_http: RwLock<reqwest::Client>,
    paused: AtomicBool,
    wake: Notify,
    shutdown: CancellationToken,
    tasks: TaskTracker,
    remote_running: Mutex<HashSet<i64>>,
    remote_progress: RwLock<HashMap<i64, DebridProgress>>,
    checker_running: AtomicBool,
    /// Serialises destination folder resolution of packages.
    dir_lock: tokio::sync::Mutex<()>,
    disk_cache: Mutex<Option<(Instant, Option<DiskInfo>)>>,
    /// Free space is below `storage.min_free_mib` (see `disk_guard.rs`).
    disk_low: AtomicBool,
    window_was_open: AtomicBool,
    /// Parallel connections accepted by hosts that refused more (learned, expires).
    connection_limits: Mutex<HashMap<String, (Instant, u32)>>,
}

impl Engine {
    pub async fn new(
        db: Db,
        config: Arc<Config>,
        settings: Arc<SettingsStore>,
        events: Arc<EventBus>,
        shutdown: CancellationToken,
    ) -> Result<Arc<Self>> {
        let api_http = reqwest::Client::builder()
            .user_agent(USER_AGENT)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(60))
            .build()?;
        let current = settings.get();
        let debrid = Arc::new(DebridManager::new(db.clone(), api_http.clone()).await?);
        let paused = db::kv_get(&db, QUEUE_PAUSED_KEY).await?.as_deref() == Some("true");
        let notifier = Notifier::new(api_http.clone());

        Ok(Arc::new(Self {
            limiter: RateLimiter::new(current.speed_limit_at(chrono::Local::now().naive_local()).0),
            download_http: RwLock::new(build_download_client(&current)?),
            extractor: Extractor::detect(config.config_dir.join("bin")),
            hosters: Hosters::default(),
            media: crate::media::MediaTools::new(config.config_dir.clone(), USER_AGENT)?,
            live: LiveRegistry::default(),
            started_at: Instant::now(),
            paused: AtomicBool::new(paused),
            wake: Notify::new(),
            tasks: TaskTracker::new(),
            remote_running: Mutex::default(),
            remote_progress: RwLock::default(),
            checker_running: AtomicBool::new(false),
            dir_lock: tokio::sync::Mutex::new(()),
            disk_cache: Mutex::new(None),
            disk_low: AtomicBool::new(false),
            window_was_open: AtomicBool::new(true),
            connection_limits: Mutex::default(),
            stats: StatsRecorder::new(db.clone()),
            db,
            config,
            settings,
            events,
            debrid,
            notifier,
            api_http,
            shutdown,
        }))
    }

    /// Runs the scheduler until shutdown, then stops every transfer cleanly.
    pub async fn run(self: Arc<Self>) {
        match db::downloads::requeue_interrupted(&self.db).await {
            Ok(0) => {}
            Ok(count) => tracing::info!("resuming {count} interrupted download(s)"),
            Err(error) => tracing::error!("cannot requeue interrupted downloads: {error:#}"),
        }
        tokio::spawn(self.clone().run_ticker());

        let mut settings_rx = self.settings.subscribe();
        loop {
            if let Err(error) = self.tick().await {
                tracing::error!("scheduler error: {error:#}");
            }
            tokio::select! {
                _ = self.shutdown.cancelled() => break,
                _ = self.wake.notified() => {}
                _ = tokio::time::sleep(Duration::from_secs(2)) => {}
                changed = settings_rx.changed() => {
                    if changed.is_ok() {
                        let settings = settings_rx.borrow_and_update().clone();
                        self.apply_settings(&settings);
                    }
                }
            }
        }

        tracing::info!("stopping downloads...");
        self.stats.flush().await;
        self.live.stop_all(StopReason::Requeue);
        self.tasks.close();
        if tokio::time::timeout(Duration::from_secs(20), self.tasks.wait()).await.is_err() {
            tracing::warn!("some tasks did not stop in time");
        }
    }

    pub fn wake(&self) {
        self.wake.notify_one();
    }

    pub fn shutdown_token(&self) -> &CancellationToken {
        &self.shutdown
    }

    pub fn download_client(&self) -> reqwest::Client {
        self.download_http.read().expect("poisoned").clone()
    }

    fn apply_settings(&self, settings: &Settings) {
        self.refresh_speed_limit(settings);
        match build_download_client(settings) {
            Ok(client) => *self.download_http.write().expect("poisoned") = client,
            Err(error) => tracing::error!("cannot rebuild HTTP client: {error}"),
        }
        self.wake();
    }

    /// Whether downloads may run right now (not paused, inside the schedule window).
    pub fn is_open(&self) -> bool {
        !self.is_paused() && self.schedule_open()
    }

    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Relaxed)
    }

    pub fn schedule_open(&self) -> bool {
        self.settings.get().schedule.is_open(chrono::Local::now().naive_local())
    }

    async fn tick(self: &Arc<Self>) -> Result<()> {
        let settings = self.settings.get();
        let open = self.is_open();
        let was_open = self.window_was_open.swap(open, Ordering::Relaxed);
        if open != was_open && !self.is_paused() {
            if open {
                tracing::info!("download window opened");
            } else {
                tracing::info!("download window closed: pausing transfers");
            }
        }

        let disk_ok = self.update_disk_guard(&settings);
        if open && disk_ok {
            let max = settings.downloads.max_parallel as usize;
            let running = self.live.running_count();
            if running < max {
                let candidates = db::downloads::next_candidates(&self.db, (max + running) as i64).await?;
                for download in candidates.into_iter().filter(|d| !self.live.is_running(d.id)).take(max - running) {
                    let live = self.live.register(download.id, download.package_id);
                    live.downloaded.store(download.downloaded.max(0) as u64, Ordering::Relaxed);
                    live.size.store(download.size.unwrap_or(0).max(0) as u64, Ordering::Relaxed);
                    self.tasks.spawn(task::run(self.clone(), download.id, live));
                }
            }
        } else if self.live.running_count() > 0 {
            self.live.stop_all(StopReason::Requeue);
        }

        self.spawn_remote_pollers().await?;
        self.spawn_extractions().await?;
        self.spawn_verifications().await?;
        self.spawn_checker();
        Ok(())
    }

    async fn spawn_remote_pollers(self: &Arc<Self>) -> Result<()> {
        for package in db::packages::list_remote(&self.db).await? {
            if self.remote_running.lock().expect("poisoned").insert(package.id) {
                self.tasks.spawn(remote::run(self.clone(), package.id));
            }
        }
        Ok(())
    }

    pub(crate) fn remote_done(&self, package_id: i64) {
        self.remote_running.lock().expect("poisoned").remove(&package_id);
        self.remote_progress.write().expect("poisoned").remove(&package_id);
    }

    pub(crate) fn set_remote_progress(&self, package_id: i64, progress: DebridProgress) {
        self.remote_progress.write().expect("poisoned").insert(package_id, progress);
    }

    pub fn remote_progress(&self, package_id: i64) -> Option<DebridProgress> {
        self.remote_progress.read().expect("poisoned").get(&package_id).cloned()
    }

    /// Status of a package for users: a running task can be ahead of the stored status.
    pub fn package_status(&self, package: &PackageRow, files: &[DownloadRow]) -> PackageStatus {
        match package.status(files) {
            PackageStatus::Queued if files.iter().any(|f| self.live.is_running(f.id)) => PackageStatus::Downloading,
            status => status,
        }
    }

    /// One extraction at a time: archives are disk-bound and parallel runs only thrash.
    async fn spawn_extractions(self: &Arc<Self>) -> Result<()> {
        if !self.live.extracting_ids().is_empty() {
            return Ok(());
        }
        if let Some(package) = db::packages::list_by_stage(&self.db, Stage::Extract).await?.into_iter().next() {
            let extraction = self.live.start_extraction(package.id, &self.shutdown);
            self.tasks.spawn(postprocess::run(self.clone(), package.id, extraction));
        }
        Ok(())
    }

    fn spawn_checker(self: &Arc<Self>) {
        if !self.checker_running.swap(true, Ordering::AcqRel) {
            let engine = self.clone();
            self.tasks.spawn(async move {
                checker::run(&engine).await;
                engine.checker_running.store(false, Ordering::Release);
            });
        }
    }

    /// Emits `stats` and `progress` once per second.
    async fn run_ticker(self: Arc<Self>) {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let mut was_busy = false;
        loop {
            tokio::select! {
                _ = self.shutdown.cancelled() => return,
                _ = interval.tick() => {}
            }
            let speed = self.live.sample();
            self.stats.sample_speed(speed).await;
            let downloads = self.live.progress();
            let packages = self.package_progress();
            let busy = !downloads.is_empty() || !packages.is_empty();
            if busy || was_busy {
                self.events.emit("progress", serde_json::json!({ "downloads": downloads, "packages": packages }));
            }
            was_busy = busy;

            let queue = db::downloads::queue_remaining_bytes(&self.db).await.unwrap_or((0, 0));
            let snapshot = Snapshot::of(&self, &downloads, queue);
            let speed_rule_active = self.refresh_speed_limit(&self.settings.get());
            self.events.emit(
                "stats",
                serde_json::json!({
                    "speed": snapshot.speed,
                    "active": snapshot.active,
                    "queued": snapshot.queued,
                    "paused": snapshot.paused,
                    "schedule_open": snapshot.schedule_open,
                    "speed_limit": self.limiter.rate().unwrap_or(0),
                    "speed_rule_active": speed_rule_active,
                    "queue_remaining": snapshot.remaining,
                    "eta": snapshot.eta(),
                    "disk_free": snapshot.disk.map(|d| d.free),
                    "disk_total": snapshot.disk.map(|d| d.total),
                    "disk_low": self.disk_low(),
                }),
            );
        }
    }

    fn package_progress(&self) -> Vec<serde_json::Value> {
        let mut ids: HashSet<i64> = self.remote_progress.read().expect("poisoned").keys().copied().collect();
        ids.extend(self.live.extracting_ids());
        ids.extend(self.live.verifying_ids());
        let mut ids: Vec<i64> = ids.into_iter().collect();
        ids.sort_unstable();
        ids.into_iter()
            .map(|id| {
                let debrid = self.remote_progress(id);
                let extract = self.live.extraction_progress(id);
                let verify = self.live.verification_progress(id);
                let status = match (extract, verify) {
                    (Some(_), _) => "extracting",
                    (None, Some(_)) => "verifying",
                    (None, None) => "debrid",
                };
                serde_json::json!({
                    "id": id,
                    "status": status,
                    "debrid": debrid,
                    "extract_progress": extract,
                    "verify_progress": verify,
                })
            })
            .collect()
    }

    /// Connections to open to `host`, given what it accepted recently.
    pub(crate) fn connection_limit(&self, host: &str) -> Option<u32> {
        let limits = self.connection_limits.lock().expect("poisoned");
        limits.get(host).filter(|(at, _)| at.elapsed() < CONNECTION_LIMIT_TTL).map(|(_, limit)| *limit)
    }

    pub(crate) fn remember_connection_limit(&self, host: &str, limit: u32) {
        let mut limits = self.connection_limits.lock().expect("poisoned");
        if limits.get(host).is_none_or(|(_, previous)| *previous != limit) {
            tracing::info!("{host} accepts {limit} parallel connection(s), adapting");
        }
        limits.insert(host.to_owned(), (Instant::now(), limit));
    }

    /// Free/total space of the download root, cached for a few seconds.
    pub fn disk_info(&self) -> Option<DiskInfo> {
        let mut cache = self.disk_cache.lock().expect("poisoned");
        if let Some((at, info)) = *cache
            && at.elapsed() < Duration::from_secs(10)
        {
            return info;
        }
        let root = &self.config.download_root;
        let info = match (fs4::available_space(root), fs4::total_space(root)) {
            (Ok(free), Ok(total)) => Some(DiskInfo { free, total }),
            _ => None,
        };
        *cache = Some((Instant::now(), info));
        info
    }

    // ----- commands -------------------------------------------------------------------------

    pub async fn set_queue_paused(&self, paused: bool) -> Result<()> {
        self.paused.store(paused, Ordering::Relaxed);
        db::kv_set(&self.db, QUEUE_PAUSED_KEY, if paused { "true" } else { "false" }).await?;
        if paused {
            self.live.stop_all(StopReason::Requeue);
        }
        tracing::info!("queue {}", if paused { "paused" } else { "resumed" });
        self.events.queue_changed();
        self.wake();
        Ok(())
    }

    pub async fn pause_package(&self, id: i64) -> Result<()> {
        db::packages::update(&self.db, id, PackageUpdate { paused: Some(true), ..Default::default() }).await?;
        db::downloads::set_status_where(&self.db, id, &[DownloadStatus::Queued], DownloadStatus::Paused).await?;
        self.live.stop_package(id, StopReason::Paused);
        self.events.queue_changed();
        Ok(())
    }

    pub async fn resume_package(&self, id: i64) -> Result<()> {
        db::packages::update(&self.db, id, PackageUpdate { paused: Some(false), ..Default::default() }).await?;
        db::downloads::set_status_where(&self.db, id, &[DownloadStatus::Paused], DownloadStatus::Queued).await?;
        self.events.queue_changed();
        self.wake();
        Ok(())
    }

    pub async fn pause_download(&self, id: i64) -> Result<()> {
        if let Some(live) = self.live.get(id) {
            live.stop(StopReason::Paused);
        } else if let Some(download) = db::downloads::get(&self.db, id).await?
            && download.status == DownloadStatus::Queued
        {
            self.set_download_status(id, DownloadStatus::Paused).await?;
        }
        self.events.queue_changed();
        Ok(())
    }

    pub async fn resume_download(&self, id: i64) -> Result<()> {
        let Some(download) = db::downloads::get(&self.db, id).await? else { return Ok(()) };
        if download.status == DownloadStatus::Paused {
            self.set_download_status(id, DownloadStatus::Queued).await?;
        }
        // Resuming a file of a paused package resumes the package, other files stay paused.
        db::packages::update(
            &self.db,
            download.package_id,
            PackageUpdate { paused: Some(false), ..Default::default() },
        )
        .await?;
        self.events.queue_changed();
        self.wake();
        Ok(())
    }

    pub async fn retry_download(&self, id: i64) -> Result<()> {
        if self.live.is_running(id) {
            return Ok(());
        }
        db::downloads::update(
            &self.db,
            id,
            DownloadUpdate {
                status: Some(DownloadStatus::Queued),
                error: Some(None),
                attempts: Some(0),
                retry_at: Some(None),
                ..Default::default()
            },
        )
        .await?;
        self.events.queue_changed();
        self.wake();
        Ok(())
    }

    pub async fn retry_package(&self, id: i64) -> Result<()> {
        let Some(package) = db::packages::get(&self.db, id).await? else { return Ok(()) };
        if package.stage == Stage::Remote && package.error.is_some() {
            remote::restart(self, &package).await?;
        }
        db::downloads::retry_failed(&self.db, id).await?;
        db::packages::update(&self.db, id, PackageUpdate { paused: Some(false), ..Default::default() }).await?;
        self.events.queue_changed();
        self.wake();
        Ok(())
    }

    /// Runs extraction again (also for packages already in the history, which the statistics
    /// count once). Only finished packages, or packages whose files are all downloaded, can be
    /// extracted; returns `false` otherwise.
    pub async fn extract_package(&self, id: i64) -> Result<bool> {
        let Some(package) = db::packages::get(&self.db, id).await? else { return Ok(false) };
        let ready = match package.stage {
            Stage::Done => true,
            Stage::Download => {
                let files = db::downloads::list_by_package(&self.db, id).await?;
                !files.is_empty() && files.iter().all(|f| f.status.is_done())
            }
            Stage::Remote | Stage::Extract | Stage::Verify => false,
        };
        if !ready {
            return Ok(false);
        }
        db::packages::update(
            &self.db,
            id,
            PackageUpdate {
                stage: Some(Stage::Extract),
                error: Some(None),
                finished_at: Some(None),
                ..Default::default()
            },
        )
        .await?;
        self.events.queue_changed();
        self.events.history_changed();
        self.wake();
        Ok(true)
    }

    async fn set_download_status(&self, id: i64, status: DownloadStatus) -> Result<()> {
        db::downloads::update(
            &self.db,
            id,
            DownloadUpdate { status: Some(status), retry_at: Some(None), ..Default::default() },
        )
        .await
    }

    /// Stops the given downloads and waits until their tasks have exited.
    async fn stop_and_wait(&self, ids: &[i64], reason: StopReason) {
        for id in ids {
            if let Some(live) = self.live.get(*id) {
                live.stop(reason);
            }
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        while ids.iter().any(|id| self.live.is_running(*id)) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    pub async fn delete_download(&self, id: i64) -> Result<()> {
        self.stop_and_wait(&[id], StopReason::Removed).await;
        // Read after stopping: the task may have created or renamed its temporary file.
        let Some(download) = db::downloads::get(&self.db, id).await? else { return Ok(()) };
        storage::remove_temp_file(&download).await;
        db::downloads::delete(&self.db, id).await?;
        if db::downloads::count_by_package(&self.db, download.package_id).await? == 0
            && let Some(package) = db::packages::get(&self.db, download.package_id).await?
            && package.stage == Stage::Download
        {
            db::packages::delete(&self.db, package.id).await?;
        } else {
            // Removing the last pending file may complete the package.
            lifecycle::on_file_settled(self, download.package_id, lifecycle::Settled::Other).await?;
        }
        self.events.queue_changed();
        self.wake();
        Ok(())
    }

    pub async fn delete_package(&self, id: i64, delete_files: bool) -> Result<()> {
        let ids: Vec<i64> = db::downloads::list_by_package(&self.db, id).await?.iter().map(|f| f.id).collect();
        self.stop_and_wait(&ids, StopReason::Removed).await;
        self.live.cancel_extraction(id);
        self.live.cancel_verification(id);
        let deadline = Instant::now() + Duration::from_secs(15);
        while (self.live.is_extracting(id) || self.live.is_verifying(id)) && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        // Read after stopping, see `delete_download`.
        let Some(package) = db::packages::get(&self.db, id).await? else { return Ok(()) };
        let files = db::downloads::list_by_package(&self.db, id).await?;
        for file in &files {
            storage::remove_temp_file(file).await;
        }
        if delete_files {
            storage::remove_package_files(&package, &files).await;
        }
        if package.stage == Stage::Remote {
            remote::forget(self, &package).await;
        }
        let _ = tokio::fs::remove_file(remote::torrent_path(&self.config.config_dir, id)).await;
        db::packages::delete(&self.db, id).await?;
        tracing::info!("package `{}` removed{}", package.name, if delete_files { " with its files" } else { "" });
        self.events.queue_changed();
        self.events.history_changed();
        self.wake();
        Ok(())
    }
}

fn build_download_client(settings: &Settings) -> reqwest::Result<reqwest::Client> {
    let timeout = Duration::from_secs(settings.downloads.timeout_secs);
    reqwest::Client::builder()
        .user_agent(settings.downloads.user_agent.clone())
        // HTTP/2 would multiplex every "connection" over one TCP stream and defeat segmentation.
        .http1_only()
        .connect_timeout(timeout)
        .pool_max_idle_per_host(32)
        .tcp_nodelay(true)
        .tcp_keepalive(Duration::from_secs(30))
        .build()
}
