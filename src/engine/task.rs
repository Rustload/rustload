//! One download, from link resolution to the final file.

use std::{
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

use tokio_util::sync::CancellationToken;

use super::{
    Engine, lifecycle,
    live::{LiveDownload, StopReason},
    m3u::{self, Playlist},
    media, naming,
    resolve::{self, Resolved},
    transfer::{Connection, Probe, ResumeState, Transfer, TransferError, TransferOptions},
};
use crate::{
    db::{self, downloads::DownloadUpdate},
    events::{Notice, NoticeLevel},
    models::{DownloadRow, DownloadStatus},
    settings::FileExists,
    util::{human_bytes, now},
};

const TEMP_SUFFIX: &str = "rlpart";
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(10);
const MAX_RETRY_DELAY_SECS: u64 = 3600;

#[derive(Debug, thiserror::Error)]
pub enum TaskError {
    #[error("stopped")]
    Stopped,
    /// The file was paused, removed or started elsewhere between scheduling and start.
    #[error("no longer queued")]
    Abandoned,
    #[error("{0}")]
    Transient(String),
    #[error("{0}")]
    Fatal(String),
}

impl From<TransferError> for TaskError {
    fn from(error: TransferError) -> Self {
        match error {
            TransferError::Cancelled => Self::Stopped,
            TransferError::Fatal(message) => Self::Fatal(message),
            other => Self::Transient(other.to_string()),
        }
    }
}

impl From<anyhow::Error> for TaskError {
    fn from(error: anyhow::Error) -> Self {
        Self::Transient(format!("{error:#}"))
    }
}

/// `Completed`: `transferred` bytes were downloaded in `elapsed` (less than `size` after a resume).
pub(super) enum Outcome {
    Completed { path: PathBuf, size: u64, transferred: u64, elapsed: Duration },
    Skipped { path: PathBuf },
}

pub async fn run(engine: Arc<Engine>, download_id: i64, live: Arc<LiveDownload>) {
    let downloaded_before = live.downloaded.load(Ordering::Relaxed);
    let outcome = execute(&engine, download_id, &live).await;
    let progressed = live.downloaded.load(Ordering::Relaxed) > downloaded_before;
    if let Err(error) = settle(&engine, download_id, &live, outcome, progressed).await {
        tracing::error!("cannot record the result of download {download_id}: {error:#}");
    }
    engine.live.unregister(download_id);
    engine.events.queue_changed();
    engine.wake();
}

async fn execute(engine: &Engine, id: i64, live: &LiveDownload) -> Result<Outcome, TaskError> {
    if !db::downloads::claim(&engine.db, id).await? {
        return Err(TaskError::Abandoned);
    }
    let download =
        db::downloads::get(&engine.db, id).await?.ok_or_else(|| TaskError::Fatal("download not found".into()))?;
    engine.events.queue_changed();
    if media::routes(engine, &download).await {
        return media::download(engine, &download, live, media::Origin::Link).await;
    }

    let cancel = live.cancel.clone();
    let mut resolved = cancellable(&cancel, resolve::resolve(engine, &download)).await??;
    let settings = engine.settings.get();
    let client = engine.download_client();
    let read_timeout = Duration::from_secs(settings.downloads.timeout_secs);

    // Debrid links can expire between runs: on 401/403 resolve once more before giving up.
    let probe = {
        let connection = Connection { client: &client, headers: &resolved.headers, read_timeout };
        match cancellable(&cancel, connection.probe(&resolved.url)).await? {
            Err(TransferError::Forbidden(_)) if !resolved.is_direct() => {
                resolved = cancellable(&cancel, resolve::resolve(engine, &download)).await??;
                let connection = Connection { client: &client, headers: &resolved.headers, read_timeout };
                cancellable(&cancel, connection.probe(&resolved.url)).await??
            }
            other => other?,
        }
    };

    let filename = naming::sanitize(&choose_filename(&download, &resolved, &probe));
    // An HLS stream nothing recognised before (no `.m3u8` in the link, playlist unreachable when
    // it was added and checked): yt-dlp takes it. Other playlists are downloaded as files.
    let playlist =
        probe.content_type.as_deref().is_some_and(m3u::is_playlist_type) || m3u::has_playlist_extension(&filename);
    if playlist
        && resolved.is_direct()
        && cancellable(&cancel, m3u::fetch(&client, &resolved.url)).await? == Some(Playlist::Stream)
    {
        drop(probe);
        tracing::info!("{} is an HLS stream: yt-dlp downloads it", download.url);
        return media::download(engine, &download, live, media::Origin::Stream).await;
    }
    if looks_like_web_page(&probe, &resolved) {
        if resolved.is_direct() && media::tries_web_pages(engine).await {
            return media::download(engine, &download, live, media::Origin::WebPage).await;
        }
        return Err(TaskError::Fatal(if resolved.is_direct() {
            "the link leads to a web page, not a file: this host needs a debrid account that supports it".into()
        } else {
            format!("{} returned a web page instead of the file", resolved.via)
        }));
    }

    let dir = download_dir(engine, &download, Some(&filename)).await?;
    create_dir(&dir).await?;

    let target = dir.join(&filename);
    if settings.downloads.file_exists == FileExists::Skip && target.exists() {
        return Ok(Outcome::Skipped { path: target });
    }

    let size = probe.size.or(resolved.size);
    if let Some(size) = size {
        check_disk_space(&dir, size.saturating_sub(download.downloaded.max(0) as u64))?;
    }

    let temp_path = dir.join(format!("{filename}.{id}.{TEMP_SUFFIX}"));
    // A previous run may have used another name or folder: start over in that case.
    let resume = match (&download.temp_path, &download.resume) {
        (Some(previous), Some(state)) if Path::new(previous) == temp_path => {
            serde_json::from_str::<ResumeState>(state).ok().filter(|state| {
                // Debrid links change server between unlocks, validators are only reliable for
                // direct links.
                !resolved.is_direct() || state.validator.is_none() || state.validator == probe.validator
            })
        }
        (Some(previous), _) => {
            let _ = tokio::fs::remove_file(previous).await;
            None
        }
        _ => None,
    };

    db::downloads::update(
        &engine.db,
        id,
        DownloadUpdate {
            status: Some(DownloadStatus::Downloading),
            filename: Some(Some(filename.clone())),
            size: Some(size.map(|s| s as i64)),
            via: Some(Some(resolved.via.clone())),
            host: Some(crate::util::url_host(&download.url)),
            temp_path: Some(Some(temp_path.to_string_lossy().into_owned())),
            expected_hash: Some(resolved.hash.as_ref().map(ToString::to_string)),
            ..Default::default()
        },
    )
    .await?;

    let min_split = u64::from(settings.downloads.min_split_size_mib) * 1024 * 1024;
    let (transfer, already) = Transfer::open(probe, &temp_path, resume, min_split)?;
    live.downloaded.store(already, Ordering::Relaxed);
    live.size.store(size.unwrap_or(0), Ordering::Relaxed);
    live.set_downloading();
    engine.events.queue_changed();
    if already > 0 {
        tracing::info!("resuming {filename} at {}", human_bytes(already));
    } else {
        tracing::info!(
            "downloading {filename} ({}) via {}",
            size.map(human_bytes).unwrap_or("unknown size".into()),
            resolved.via
        );
    }

    let started = Instant::now();
    let server = crate::util::url_host(&resolved.url).unwrap_or_default();
    let max_connections = match engine.connection_limit(&server) {
        Some(limit) => settings.downloads.connections.min(limit),
        None => settings.downloads.connections,
    };
    let options = TransferOptions {
        connection: Connection { client: &client, headers: &resolved.headers, read_timeout },
        max_connections,
        limiter: &engine.limiter,
        live,
        decryptor: resolved.decryptor.as_deref(),
    };
    let result = run_with_checkpoints(engine, id, &transfer, &options, &cancel).await;
    if let Some(limit) = transfer.connection_limit() {
        engine.remember_connection_limit(&server, limit);
    }
    let final_size = match result {
        Ok(size) => size,
        Err(error) => {
            save_checkpoint(engine, id, &transfer).await;
            return Err(error.into());
        }
    };
    drop(transfer);

    let path = finalize(&temp_path, &dir, &filename, settings.downloads.file_exists).await?;
    Ok(Outcome::Completed {
        path,
        size: final_size,
        transferred: final_size.saturating_sub(already),
        elapsed: started.elapsed(),
    })
}

/// Runs the transfer while persisting its resume state every few seconds, concurrently so that
/// slow disk flushes never stall the network.
async fn run_with_checkpoints(
    engine: &Engine,
    id: i64,
    transfer: &Transfer,
    options: &TransferOptions<'_>,
    cancel: &CancellationToken,
) -> Result<u64, TransferError> {
    let done = CancellationToken::new();
    let (result, ()) = tokio::join!(
        async {
            let result = transfer.run(options, cancel).await;
            done.cancel();
            result
        },
        async {
            loop {
                tokio::select! {
                    _ = done.cancelled() => break,
                    _ = tokio::time::sleep(CHECKPOINT_INTERVAL) => save_checkpoint(engine, id, transfer).await,
                }
            }
        }
    );
    result
}

async fn save_checkpoint(engine: &Engine, id: i64, transfer: &Transfer) {
    let Some(state) = transfer.checkpoint().await else { return };
    let update = DownloadUpdate {
        downloaded: Some(state.downloaded() as i64),
        resume: Some(serde_json::to_string(&state).ok()),
        ..Default::default()
    };
    if let Err(error) = db::downloads::update(&engine.db, id, update).await {
        tracing::warn!("cannot save resume state: {error:#}");
    }
}

/// Best file name: provider > Content-Disposition > link check > URL.
fn choose_filename(download: &DownloadRow, resolved: &Resolved, probe: &Probe) -> String {
    resolved
        .filename
        .clone()
        .or_else(|| probe.filename.clone())
        .or_else(|| download.filename.clone())
        .or_else(|| naming::from_url(&probe.final_url))
        .or_else(|| naming::from_url(&download.url))
        .unwrap_or_else(|| format!("download-{}", download.id))
}

/// A web page where a file was expected. A page is kept only when the server or the debrid service
/// names it `.html` (a saved page, an export): a link that merely ends in `.html` is the download
/// page of many file hosts.
fn looks_like_web_page(probe: &Probe, resolved: &Resolved) -> bool {
    let named_html = resolved
        .filename
        .as_deref()
        .or(probe.filename.as_deref())
        .and_then(naming::extension)
        .is_some_and(|ext| matches!(ext.as_str(), "html" | "htm" | "xhtml"));
    probe.is_html() && !named_html
}

/// Folder of a file: its package folder, or a sub-folder of it. `filename` is the name the file
/// gets, when it is not stored yet.
pub(super) async fn download_dir(
    engine: &Engine,
    download: &DownloadRow,
    filename: Option<&str>,
) -> Result<PathBuf, TaskError> {
    let starting = filename.map(|name| (download.id, name));
    Ok(engine.package_dir(download.package_id, starting).await?.join(naming::sanitize_relative_dir(&download.rel_dir)))
}

pub(super) async fn create_dir(dir: &Path) -> Result<(), TaskError> {
    tokio::fs::create_dir_all(dir)
        .await
        .map_err(|e| TaskError::Fatal(format!("cannot create folder {}: {e}", dir.display())))
}

pub(super) fn check_disk_space(dir: &Path, needed: u64) -> Result<(), TaskError> {
    match fs4::available_space(dir) {
        Ok(free) if free < needed => Err(TaskError::Fatal(format!(
            "not enough disk space: {} needed, {} free",
            human_bytes(needed),
            human_bytes(free)
        ))),
        _ => Ok(()),
    }
}

/// Moves the temporary file to its final name according to the file-exists policy.
pub(super) async fn finalize(
    temp: &Path,
    dir: &Path,
    filename: &str,
    policy: FileExists,
) -> Result<PathBuf, TaskError> {
    let target = match policy {
        FileExists::Overwrite | FileExists::Skip => dir.join(filename),
        FileExists::Rename => naming::unique_path(dir, filename),
    };
    if policy == FileExists::Overwrite && target.exists() {
        let _ = tokio::fs::remove_file(&target).await;
    }
    tokio::fs::rename(temp, &target)
        .await
        .map_err(|e| TaskError::Fatal(format!("cannot move the file into place: {e}")))?;
    Ok(target)
}

/// Records the outcome in the database and moves the package forward.
async fn settle(
    engine: &Engine,
    id: i64,
    live: &LiveDownload,
    outcome: Result<Outcome, TaskError>,
    progressed: bool,
) -> anyhow::Result<()> {
    if matches!(outcome, Err(TaskError::Abandoned)) {
        return Ok(());
    }
    let Some(download) = db::downloads::get(&engine.db, id).await? else { return Ok(()) };
    let name = download.filename.clone().unwrap_or_else(|| download.url.clone());
    let settings = engine.settings.get();

    let update = match outcome {
        Ok(Outcome::Completed { path, size, transferred, elapsed }) => {
            let speed = size as f64 / elapsed.as_secs_f64().max(0.001);
            tracing::info!(
                "finished {name} ({}) in {:.0?}, {}/s",
                human_bytes(size),
                elapsed,
                human_bytes(speed as u64)
            );
            engine
                .stats
                .file_completed(id, download.host.as_deref(), download.via.as_deref(), size, transferred, elapsed)
                .await;
            DownloadUpdate {
                status: Some(DownloadStatus::Completed),
                path: Some(Some(path.to_string_lossy().into_owned())),
                filename: Some(path.file_name().map(|n| n.to_string_lossy().into_owned())),
                size: Some(Some(size as i64)),
                downloaded: Some(size as i64),
                temp_path: Some(None),
                resume: Some(None),
                error: Some(None),
                attempts: Some(0),
                finished_at: Some(Some(now())),
                // New content: any previous verification result is obsolete.
                checksum_algo: Some(None),
                checksum_status: Some(None),
                ..Default::default()
            }
        }
        Ok(Outcome::Skipped { path }) => {
            tracing::info!("skipped {name}: {} already exists", path.display());
            DownloadUpdate {
                status: Some(DownloadStatus::Skipped),
                path: Some(Some(path.to_string_lossy().into_owned())),
                error: Some(Some("file already exists".into())),
                finished_at: Some(Some(now())),
                ..Default::default()
            }
        }
        Err(TaskError::Stopped) => {
            let status = match live.stop_reason() {
                Some(StopReason::Removed) => return Ok(()),
                Some(StopReason::Paused) => DownloadStatus::Paused,
                _ => DownloadStatus::Queued,
            };
            DownloadUpdate { status: Some(status), ..Default::default() }
        }
        // A run that made progress starts the count again: only consecutive failures matter.
        Err(TaskError::Transient(message))
            if progressed || download.attempts < i64::from(settings.downloads.max_retries) =>
        {
            let attempts = if progressed { 1 } else { download.attempts + 1 };
            let delay = settings
                .downloads
                .retry_delay_secs
                .saturating_mul(1 << (attempts - 1).min(6))
                .min(MAX_RETRY_DELAY_SECS);
            tracing::warn!("{name}: {message} — retry {attempts}/{} in {delay}s", settings.downloads.max_retries);
            DownloadUpdate {
                status: Some(DownloadStatus::Queued),
                error: Some(Some(message)),
                attempts: Some(attempts),
                retry_at: Some(Some(now() + delay as i64)),
                ..Default::default()
            }
        }
        Err(TaskError::Abandoned) => return Ok(()),
        Err(TaskError::Transient(message) | TaskError::Fatal(message)) => {
            tracing::error!("{name} failed: {message}");
            engine.events.notice(
                Notice::new(NoticeLevel::Error, "download_failed", format!("{name} failed: {message}"))
                    .param("name", &name)
                    .param("error", &message),
            );
            DownloadUpdate {
                status: Some(DownloadStatus::Failed),
                error: Some(Some(message)),
                retry_at: Some(None),
                ..Default::default()
            }
        }
    };
    let cause = match update.status {
        Some(DownloadStatus::Failed) => Some(lifecycle::Settled::Failed),
        Some(DownloadStatus::Completed | DownloadStatus::Skipped) => Some(lifecycle::Settled::Other),
        _ => None,
    };
    db::downloads::update(&engine.db, id, update).await?;
    if let Some(cause) = cause {
        lifecycle::on_file_settled(engine, download.package_id, cause).await?;
    }

    Ok(())
}

/// Awaits `future` unless the download is stopped first.
async fn cancellable<T>(cancel: &CancellationToken, future: impl Future<Output = T>) -> Result<T, TaskError> {
    tokio::select! {
        _ = cancel.cancelled() => Err(TaskError::Stopped),
        value = future => Ok(value),
    }
}
