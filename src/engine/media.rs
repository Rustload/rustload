//! Media links in the queue: routing to yt-dlp, link checks, playlists, the download task and
//! the managed yt-dlp updates. The yt-dlp layer itself is `crate::media`.
//!
//! A media download runs yt-dlp in a private work folder next to its destination
//! (`.rustload-media-<id>`): partial files, separate video/audio streams and intermediate files
//! never appear under their real name, resuming relies on yt-dlp's `.part` files kept there, and
//! the finished file is moved into place with the same file-exists policy as any download.

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
    time::{Duration, Instant},
};

use tokio_util::sync::CancellationToken;

use super::{
    Engine,
    handler::Handler,
    lifecycle,
    live::LiveDownload,
    m3u, naming,
    task::{self, Outcome, TaskError},
};
use crate::{
    db::{self, downloads::DownloadUpdate, media::NewEntry, packages::PackageUpdate},
    events::{Notice, NoticeLevel},
    media::{
        self, MediaError, MediaInfo, MediaMeta, MediaOptions, MediaOptionsPatch, YtDlp,
        info::{MAX_ENTRIES, MediaEntry, RawInfo, clip_title, predicted_name},
        output::{self, ErrorKind, Line, ProgressTracker},
        process::Running,
    },
    models::{DownloadRow, DownloadStatus, PackageRow},
    settings::{FileExists, Settings},
    util::{now, url_host},
};

/// `via` of downloads handled by yt-dlp.
const VIA: &str = "yt-dlp";
const WORK_DIR_PREFIX: &str = ".rustload-media-";
/// Same as Rustload's own names, made safe for Windows/SMB shares by `--windows-filenames`.
const OUTPUT_TEMPLATE: &str = "%(title).180B [%(id)s].%(ext)s";
/// Listing a playlist can take several requests.
const EXPAND_TIMEOUT: Duration = Duration::from_secs(60);
const LIVE_UNSUPPORTED: &str = "live streams are not supported: download the recording once the stream has ended";
const PLAYLIST_IN_FILE: &str = "this link is a playlist or a channel: add it again to list its videos";
const UPDATE_STARTUP_DELAY: Duration = Duration::from_secs(120);
const UPDATE_INTERVAL: Duration = Duration::from_secs(3600);
const UPDATE_EVERY_SECS: i64 = 24 * 3600;

/// How a download reached yt-dlp.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Origin {
    /// A link of a known media site (or one already handled by yt-dlp).
    Link,
    /// A web page that nothing else recognised (`media.generic_pages`).
    WebPage,
    /// An HLS stream recognised from its content when its download started (`engine::m3u`).
    Stream,
}

/// Whether a download goes to yt-dlp instead of debrid accounts, hoster plugins or a direct
/// transfer: links found by yt-dlp (playlist entries, downloads it started), HLS streams (their
/// `media` is set when they are recognised, see `engine::m3u`) and links of media sites, unless
/// yt-dlp is missing and something else handles the site (see [`takes_site_link`]).
pub(super) async fn routes(engine: &Engine, download: &DownloadRow) -> bool {
    if !engine.settings.get().media.enabled {
        return false;
    }
    if download.media.is_some() || download.via.as_deref() == Some(VIA) {
        return true;
    }
    if media::detect_str(&download.url).is_none() {
        return false;
    }
    let takes = takes_site_link(engine, &download.url).await;
    if !takes {
        tracing::info!(
            "yt-dlp is not installed: {} goes to the debrid account or hoster plugin supporting the site",
            download.url
        );
    }
    takes
}

/// Whether a link of a media site goes to yt-dlp. Without yt-dlp, a debrid account or a hoster
/// plugin supporting the site takes it, as before media sites were handled; links nothing else
/// handles stay with yt-dlp, whose absence is the error then reported.
pub(super) async fn takes_site_link(engine: &Engine, url: &str) -> bool {
    let settings = engine.settings.get();
    engine.media.tools(&settings.media).await.ytdlp.is_some()
        || matches!(Handler::select(engine, url).await, Handler::Direct)
}

/// Whether web pages that nothing else handles should be tried with yt-dlp.
pub(super) async fn tries_web_pages(engine: &Engine) -> bool {
    let settings = engine.settings.get();
    settings.media.enabled && settings.media.generic_pages && engine.media.tools(&settings.media).await.ytdlp.is_some()
}

fn work_dir(dir: &Path, download_id: i64) -> PathBuf {
    dir.join(format!("{WORK_DIR_PREFIX}{download_id}"))
}

/// Whether `path` is the work folder of a media download (removed with the download).
pub(super) fn is_work_dir(path: &Path) -> bool {
    path.file_name().is_some_and(|name| name.to_string_lossy().starts_with(WORK_DIR_PREFIX))
}

/// Options of a package: its own, or the settings defaults.
pub(crate) fn package_options(settings: &Settings, package: &PackageRow) -> MediaOptions {
    package.media_options().unwrap_or_else(|| settings.media.defaults.clone())
}

/// Options for new packages: the request's, over the settings defaults.
pub(super) fn options_for(settings: &Settings, patch: Option<&MediaOptionsPatch>) -> Result<MediaOptions, String> {
    let base = &settings.media.defaults;
    patch.map_or_else(|| base.clone(), |patch| patch.apply(base)).validate()
}

/// Stores what a probe told about a link: metadata, a provisional file name (which also lets the
/// automatic category pick Videos or Music) and the size estimate.
async fn record_probe(engine: &Engine, download: &DownloadRow, info: &MediaInfo, options: &MediaOptions, ffmpeg: bool) {
    let meta = info.meta();
    let ext = options.expected_extension(ffmpeg, info.ext.as_deref());
    let filename = predicted_name(&meta, &ext).map(|name| naming::sanitize(&name));
    let size = info.filesize_approx.map(|s| s as i64);
    let host = url_host(&download.url);
    let stored = async {
        db::media::set_download_meta(&engine.db, download.id, &meta).await?;
        db::downloads::record_check(&engine.db, download.id, filename.as_deref(), size, host.as_deref()).await
    };
    if let Err(error) = stored.await {
        tracing::warn!("cannot store media details: {error:#}");
    }
}

/// Names a package after the first video probed in it, unless the user named it or its folder
/// already exists.
async fn name_package(engine: &Engine, package_id: i64, title: Option<&str>) {
    let Some(title) = title.map(str::trim).filter(|t| !t.is_empty()) else { return };
    let Ok(Some(package)) = db::packages::get(&engine.db, package_id).await else { return };
    if package.name_locked || package.dest_dir.is_some() {
        return;
    }
    let update = PackageUpdate { name: Some(title.to_owned()), name_locked: Some(true), ..Default::default() };
    if let Err(error) = db::packages::update(&engine.db, package_id, update).await {
        tracing::warn!("cannot rename package: {error:#}");
    }
}

/// Download rows for playlist entries.
fn entry_rows(entries: &[MediaEntry], options: &MediaOptions, ffmpeg: bool) -> Vec<NewEntry> {
    let ext = options.expected_extension(ffmpeg, None);
    entries
        .iter()
        .map(|entry| NewEntry {
            url: entry.url.clone(),
            filename: predicted_name(&entry.meta, &ext).map(|name| naming::sanitize(&name)),
            host: url_host(&entry.url),
            meta: entry.meta.clone(),
        })
        .collect()
}

/// The videos of a playlist, with lists nested in it (channel tabs) expanded one level.
async fn playlist_entries(ytdlp: &YtDlp, info: MediaInfo, engine: &Engine) -> Vec<MediaEntry> {
    let mut entries = Vec::new();
    for entry in info.entries {
        if entries.len() >= MAX_ENTRIES {
            break;
        }
        if !entry.nested {
            entries.push(entry);
            continue;
        }
        match ytdlp.probe(&entry.url, None, EXPAND_TIMEOUT, engine.shutdown_token()).await {
            Ok(nested) => entries.extend(nested.entries.into_iter().filter(|e| !e.nested)),
            Err(error) => tracing::warn!("cannot list {}: {error}", entry.url),
        }
    }
    entries.truncate(MAX_ENTRIES);
    entries
}

/// A playlist, channel or album link listed when added.
pub(super) struct Collection {
    pub url: String,
    pub title: Option<String>,
    pub entries: Vec<NewEntry>,
}

/// Lists the playlists among `links`; other links (and playlists that cannot be listed, whose
/// error then shows on the file) are returned as they are.
pub(super) async fn expand_collections(
    engine: &Engine,
    links: Vec<String>,
    options: &MediaOptions,
) -> (Vec<Collection>, Vec<String>) {
    let candidates = links.iter().any(|l| media::detect_str(l).is_some_and(|site| site.collection));
    if !candidates {
        return (Vec::new(), links);
    }
    let Ok(ytdlp) = YtDlp::current(engine).await else { return (Vec::new(), links) };
    let mut collections = Vec::new();
    let mut others = Vec::new();
    for link in links {
        if !media::detect_str(&link).is_some_and(|site| site.collection) {
            others.push(link);
            continue;
        }
        match ytdlp.probe(&link, None, EXPAND_TIMEOUT, engine.shutdown_token()).await {
            Ok(info) if info.is_playlist() => {
                let title = info.title.clone();
                let entries = playlist_entries(&ytdlp, info, engine).await;
                if entries.is_empty() {
                    tracing::warn!("{link}: the playlist is empty");
                    others.push(link);
                } else {
                    let entries = entry_rows(&entries, options, ytdlp.has_ffmpeg());
                    collections.push(Collection { url: link, title, entries });
                }
            }
            Ok(_) => others.push(link),
            Err(error) => {
                tracing::warn!("cannot list {link}: {error}");
                others.push(link);
            }
        }
    }
    (collections, others)
}

/// Availability check of a media link: title, duration and size estimate through yt-dlp
/// instead of an HTTP request. Returns `false` when the link is not one for yt-dlp.
pub(super) async fn check(engine: &Engine, download: &DownloadRow) -> bool {
    let settings = engine.settings.get();
    if download.account_id.is_some() || engine.live.is_running(download.id) || !routes(engine, download).await {
        return false;
    }
    let Ok(ytdlp) = YtDlp::current(engine).await else { return false };
    let Ok(Some(package)) = db::packages::get(&engine.db, download.package_id).await else { return false };
    let options = package_options(&settings, &package);
    let probe = ytdlp.probe(&download.url, Some(&options), media::PROBE_TIMEOUT, engine.shutdown_token()).await;
    let result = match probe {
        Ok(info) if info.is_playlist() => expand_in_place(engine, &ytdlp, download, &options, info).await,
        Ok(info) if info.is_live => mark_failed(engine, download, LIVE_UNSUPPORTED).await,
        Ok(info) => {
            record_probe(engine, download, &info, &options, ytdlp.has_ffmpeg()).await;
            name_package(engine, download.package_id, info.title.as_deref()).await;
            Ok(())
        }
        Err(MediaError::Unavailable(message)) if output::classify(&message) == ErrorKind::Permanent => {
            let reason = explain(&message, ytdlp.has_ffmpeg(), Origin::Link, engine.config.desktop);
            tracing::warn!("media link unavailable: {} ({reason})", download.url);
            mark_failed(engine, download, &reason).await
        }
        Err(error) => {
            tracing::debug!("media link check of {} inconclusive: {error}", download.url);
            db::downloads::record_check(&engine.db, download.id, None, None, url_host(&download.url).as_deref()).await
        }
    };
    if let Err(error) = result {
        tracing::warn!("cannot store link check: {error:#}");
    }
    true
}

async fn mark_failed(engine: &Engine, download: &DownloadRow, reason: &str) -> anyhow::Result<()> {
    if !db::downloads::mark_offline(&engine.db, download.id, reason).await? {
        return Ok(());
    }
    lifecycle::on_file_settled(engine, download.package_id, lifecycle::Settled::Failed).await
}

/// A link added as a single file turned out to be a playlist: its entries replace it.
async fn expand_in_place(
    engine: &Engine,
    ytdlp: &YtDlp,
    download: &DownloadRow,
    options: &MediaOptions,
    info: MediaInfo,
) -> anyhow::Result<()> {
    let title = info.title.clone();
    let entries = playlist_entries(ytdlp, info, engine).await;
    if entries.is_empty() {
        return mark_failed(engine, download, "the playlist is empty").await;
    }
    let rows = entry_rows(&entries, options, ytdlp.has_ffmpeg());
    if db::media::replace_with_entries(&engine.db, download.id, download.package_id, &rows).await? {
        tracing::info!("{} listed: {} video(s)", download.url, rows.len());
        name_package(engine, download.package_id, title.as_deref()).await;
        engine.events.queue_changed();
        engine.wake();
    }
    Ok(())
}

/// User-facing explanation of a yt-dlp failure. `desktop`: in the desktop app, where ffmpeg does
/// not come with a Docker image.
fn explain(message: &str, ffmpeg: bool, origin: Origin, desktop: bool) -> String {
    let lower = message.to_ascii_lowercase();
    if origin == Origin::WebPage && lower.contains("unsupported url") {
        return "the link leads to a web page, not a file, and yt-dlp found no video on it".into();
    }
    if !ffmpeg && lower.contains("requested format is not available") {
        let install = match (desktop, media::tools::can_install_ffmpeg()) {
            (false, _) => "install ffmpeg (included in the Docker image)",
            (true, true) => "install ffmpeg in Settings › Video sites",
            (true, false) => "install ffmpeg with your package manager",
        };
        return format!(
            "this video only exists as separate video and audio streams, which need ffmpeg to be combined: \
             {install} or choose audio only"
        );
    }
    let text = output::clean_error(message);
    if output::needs_login(message) {
        format!("{text} (a cookies file can be set in Settings › Video sites)")
    } else {
        text
    }
}

fn task_error(error: MediaError, ffmpeg: bool, origin: Origin, desktop: bool) -> TaskError {
    match error {
        MediaError::Cancelled => TaskError::Stopped,
        MediaError::Unavailable(message) => failure(&message, ffmpeg, origin, desktop),
        MediaError::Disabled if origin == Origin::Stream => TaskError::Fatal(m3u::NEEDS_YTDLP.into()),
        error @ (MediaError::Disabled | MediaError::Missing) => TaskError::Fatal(error.to_string()),
        error => TaskError::Transient(error.to_string()),
    }
}

/// A yt-dlp error: permanent ones are explained, others retried.
fn failure(message: &str, ffmpeg: bool, origin: Origin, desktop: bool) -> TaskError {
    match output::classify(message) {
        ErrorKind::Permanent => TaskError::Fatal(explain(message, ffmpeg, origin, desktop)),
        ErrorKind::Retryable => TaskError::Transient(output::clean_error(message)),
    }
}

/// Per-download share of the speed limit in effect (global limit or speed rule). yt-dlp traffic
/// does not go through Rustload's limiter: the limit is split evenly between running downloads
/// when each starts.
fn rate_limit(limit: Option<u64>, running: usize) -> Option<u64> {
    limit.map(|limit| (limit / running.max(1) as u64).max(1024))
}

/// Downloads a media link with yt-dlp.
pub(super) async fn download(
    engine: &Engine,
    download: &DownloadRow,
    live: &LiveDownload,
    origin: Origin,
) -> Result<Outcome, TaskError> {
    let desktop = engine.config.desktop;
    let ytdlp = YtDlp::current(engine).await.map_err(|e| task_error(e, false, origin, desktop))?;
    let ffmpeg = ytdlp.has_ffmpeg();
    let settings = engine.settings.get();
    let cancel = live.cancel.clone();
    let package = db::packages::get(&engine.db, download.package_id)
        .await?
        .ok_or_else(|| TaskError::Fatal("package not found".into()))?;
    let options = package_options(&settings, &package);
    if package.media.is_none() {
        // Reached through `generic_pages` or as a stream: show the options used.
        db::media::set_package_options(&engine.db, package.id, Some(&options)).await?;
    }

    // The folder may be named after the video: know it before resolving the folder.
    let meta = match download.media_meta() {
        Some(meta) => meta,
        None => {
            let info = ytdlp
                .probe(&download.url, Some(&options), media::PROBE_TIMEOUT, &cancel)
                .await
                .map_err(|e| task_error(e, ffmpeg, origin, desktop))?;
            if info.is_playlist() {
                return Err(TaskError::Fatal(PLAYLIST_IN_FILE.into()));
            }
            if info.is_live {
                return Err(TaskError::Fatal(LIVE_UNSUPPORTED.into()));
            }
            record_probe(engine, download, &info, &options, ffmpeg).await;
            info.meta()
        }
    };
    name_package(engine, package.id, meta.title.as_deref()).await;
    let label = meta.title.clone().unwrap_or_else(|| download.url.clone());

    // The probe stored the file name.
    let dir = task::download_dir(engine, download, None).await?;
    let work = work_dir(&dir, download.id);
    task::create_dir(&work).await?;
    let known_size = db::downloads::get(&engine.db, download.id).await?.and_then(|d| d.size).map(|s| s.max(0) as u64);
    if let Some(size) = known_size {
        task::check_disk_space(&dir, size.saturating_sub(download.downloaded.max(0) as u64))?;
    }

    db::downloads::update(
        &engine.db,
        download.id,
        DownloadUpdate {
            status: Some(DownloadStatus::Downloading),
            via: Some(Some(VIA.into())),
            host: Some(url_host(&download.url)),
            temp_path: Some(Some(work.to_string_lossy().into_owned())),
            ..Default::default()
        },
    )
    .await?;
    live.connections.store(1, Ordering::Relaxed);
    engine.events.queue_changed();

    let plan = options.plan(ffmpeg);
    for note in &plan.notes {
        tracing::warn!("{label}: {note}");
    }
    let cookies = ytdlp.cookies().map_err(|e| TaskError::Fatal(e.to_string()))?;
    let mut args = ytdlp.base_args(cookies.as_ref());
    args.extend(output::output_args().into_iter().map(OsString::from));
    args.extend(["--windows-filenames", "-o"].map(OsString::from));
    args.push(output_template(&meta).into());
    args.push("-P".into());
    args.push(work.clone().into());
    let limit = settings.speed_limit_at(chrono::Local::now().naive_local()).0;
    if let Some(rate) = rate_limit(limit, engine.live.running_count()) {
        args.extend(["--limit-rate".into(), rate.to_string().into()]);
    }
    args.extend(plan.args().map(OsString::from));
    args.extend(ytdlp.tail_args(&download.url));

    tracing::info!("downloading {label} with yt-dlp");
    let started = Instant::now();
    let resumed_from = live.downloaded.load(Ordering::Relaxed);
    let mut running = Running::spawn(&ytdlp.program, &args, &ytdlp.tmp_dir)
        .map_err(|e| TaskError::Fatal(format!("cannot start yt-dlp: {e}")))?;
    let policy = settings.downloads.file_exists;
    // With the skip policy, stop as soon as the name is known if the file exists.
    let existing = |info: &RawInfo| {
        (policy == FileExists::Skip)
            .then(|| expected_path(&dir, info, &options, ffmpeg))
            .flatten()
            .filter(|p| p.exists())
    };
    let run = follow(&mut running, live, &cancel, known_size, existing).await;
    let status = running.wait().await;
    drop(cookies);

    let file = match run {
        Ok(Followed::Finished { file, error, stderr }) => match (status, file) {
            (Ok(status), Some(file)) if status.success() => file,
            (Ok(status), None) if status.success() => {
                save_progress(engine, download.id, live).await;
                return Err(TaskError::Transient("yt-dlp finished without producing a file".into()));
            }
            (Ok(status), _) => {
                let message = error.or(stderr).unwrap_or_else(|| format!("yt-dlp exited with {status}"));
                save_progress(engine, download.id, live).await;
                return Err(failure(&message, ffmpeg, origin, desktop));
            }
            (Err(error), _) => return Err(TaskError::Transient(format!("yt-dlp did not finish: {error}"))),
        },
        Ok(Followed::Exists(path)) => {
            remove_work_dir(&work).await;
            return Ok(Outcome::Skipped { path });
        }
        Err(error) => {
            save_progress(engine, download.id, live).await;
            return Err(error);
        }
    };

    let outcome = publish(&work, &file, &dir, policy).await?;
    remove_work_dir(&work).await;
    Ok(match outcome {
        Published::Moved { path, size } => {
            // yt-dlp resumes on its own: count what this run fetched, not the whole file.
            let transferred = live.downloaded.load(Ordering::Relaxed).saturating_sub(resumed_from).min(size);
            Outcome::Completed { path, size, transferred, elapsed: started.elapsed() }
        }
        Published::Exists(path) => Outcome::Skipped { path },
    })
}

enum Followed {
    /// yt-dlp closed its output: the final file if it printed one, and its last error.
    Finished { file: Option<PathBuf>, error: Option<String>, stderr: Option<String> },
    /// The file already exists and the policy is to skip it (yt-dlp was stopped).
    Exists(PathBuf),
}

/// Reads yt-dlp's output until it exits, feeding the live counters. `existing` tells, once the
/// video is known, whether its file already exists and must not be downloaded.
async fn follow(
    running: &mut Running,
    live: &LiveDownload,
    cancel: &CancellationToken,
    known_size: Option<u64>,
    existing: impl Fn(&RawInfo) -> Option<PathBuf>,
) -> Result<Followed, TaskError> {
    let mut tracker = ProgressTracker::new(live.downloaded.load(Ordering::Relaxed));
    let mut file = None;
    let mut error = None;
    let mut stderr = None;
    loop {
        let line = tokio::select! {
            _ = cancel.cancelled() => {
                running.kill().await;
                return Err(TaskError::Stopped);
            }
            line = running.next_line() => line,
        };
        match line {
            None => return Ok(Followed::Finished { file, error, stderr }),
            Some(Line::Info(info)) => {
                if info.is_live() {
                    running.kill().await;
                    return Err(TaskError::Fatal(LIVE_UNSUPPORTED.into()));
                }
                if let Some(path) = existing(&info) {
                    running.kill().await;
                    return Ok(Followed::Exists(path));
                }
                tracker.plan_single(&info);
                if let Some(total) = tracker.total().or(known_size) {
                    live.size.store(total, Ordering::Relaxed);
                }
            }
            Some(Line::Formats(formats)) if !formats.is_empty() => tracker.plan(&formats),
            Some(Line::Progress(progress)) => {
                tracker.update(&progress);
                let downloaded = tracker.downloaded();
                live.downloaded.store(downloaded, Ordering::Relaxed);
                if let Some(total) = tracker.total().or(known_size) {
                    live.size.store(total.max(downloaded), Ordering::Relaxed);
                }
                live.set_downloading();
            }
            Some(Line::File(path)) => file = Some(path),
            Some(Line::Error(message)) => {
                tracing::debug!("yt-dlp: ERROR: {message}");
                error = Some(message);
            }
            Some(Line::Warning(message)) => tracing::debug!("yt-dlp: WARNING: {message}"),
            Some(Line::Stderr(text)) if !text.trim().is_empty() => {
                tracing::debug!("yt-dlp: {text}");
                stderr = Some(text);
            }
            Some(Line::Other(text)) if !text.trim().is_empty() => tracing::debug!("yt-dlp: {text}"),
            Some(_) => {}
        }
    }
}

/// yt-dlp's file names (see `predicted_name`): `title [id]`, or the title alone for links named
/// after their URL, which have no id (`MediaInfo::name_after_url`).
fn output_template(meta: &MediaMeta) -> String {
    match (&meta.id, meta.title.as_deref().map(str::trim).filter(|t| !t.is_empty())) {
        (None, Some(title)) => format!("{}.%(ext)s", naming::sanitize(clip_title(title)).replace('%', "%%")),
        _ => OUTPUT_TEMPLATE.to_owned(),
    }
}

/// Where the file will end up, from what yt-dlp announced before downloading.
fn expected_path(dir: &Path, info: &RawInfo, options: &MediaOptions, ffmpeg: bool) -> Option<PathBuf> {
    let name = Path::new(info.filename.as_deref()?).file_name()?.to_string_lossy().into_owned();
    let (stem, ext) = naming::split_extension(&name);
    let ext = options.expected_extension(ffmpeg, ext.or(info.ext.as_deref()));
    Some(dir.join(naming::sanitize(&format!("{stem}.{ext}"))))
}

async fn save_progress(engine: &Engine, id: i64, live: &LiveDownload) {
    let update = DownloadUpdate {
        downloaded: Some(live.downloaded.load(Ordering::Relaxed) as i64),
        size: live.size().map(|s| Some(s as i64)),
        ..Default::default()
    };
    if let Err(error) = db::downloads::update(&engine.db, id, update).await {
        tracing::warn!("cannot save media progress: {error:#}");
    }
}

async fn remove_work_dir(work: &Path) {
    if let Err(error) = tokio::fs::remove_dir_all(work).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("cannot remove {}: {error}", work.display());
    }
}

enum Published {
    Moved { path: PathBuf, size: u64 },
    Exists(PathBuf),
}

/// Moves the finished file (and side files such as subtitles) from the work folder to `dir`,
/// with the file-exists policy and Rustload's name sanitising.
async fn publish(work: &Path, file: &Path, dir: &Path, policy: FileExists) -> Result<Published, TaskError> {
    let file = if file.is_absolute() { file.to_path_buf() } else { work.join(file) };
    if !file.is_file() || file.parent() != Some(work) {
        return Err(TaskError::Fatal(format!("yt-dlp reported an unexpected file: {}", file.display())));
    }
    let original = file.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let name = naming::sanitize(&original);
    if policy == FileExists::Skip && dir.join(&name).exists() {
        return Ok(Published::Exists(dir.join(name)));
    }
    let path = task::finalize(&file, dir, &name, policy).await?;
    let size = tokio::fs::metadata(&path).await.map(|m| m.len()).unwrap_or(0);

    let original_stem = naming::split_extension(&original).0.to_owned();
    let final_name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let final_stem = naming::split_extension(&final_name).0.to_owned();
    for side in side_files(work).await {
        let side_name = side.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let renamed = match side_name.strip_prefix(&original_stem) {
            Some(rest) => format!("{final_stem}{rest}"),
            None => side_name,
        };
        let renamed = naming::sanitize(&renamed);
        if policy == FileExists::Skip && dir.join(&renamed).exists() {
            continue;
        }
        if let Err(error) = task::finalize(&side, dir, &renamed, policy).await {
            tracing::warn!("cannot move {}: {error}", side.display());
        }
    }
    Ok(Published::Moved { path, size })
}

/// Files left in the work folder next to the main file (subtitles, thumbnails...), without
/// yt-dlp's temporary files.
async fn side_files(work: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let Ok(mut entries) = tokio::fs::read_dir(work).await else { return files };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let name = entry.file_name().to_string_lossy().to_ascii_lowercase();
        let temporary =
            [".part", ".ytdl", ".temp", ".tmp"].iter().any(|ext| name.ends_with(ext)) || name.contains(".part-");
        if !temporary && entry.file_type().await.is_ok_and(|t| t.is_file()) {
            files.push(entry.path());
        }
    }
    files
}

/// Startup detection, then a daily check for yt-dlp releases (updating the managed copy).
pub async fn maintain(engine: Arc<Engine>) {
    let settings = engine.settings.get();
    if settings.media.enabled {
        let tools = engine.media.tools(&settings.media).await;
        match &tools.ytdlp {
            Some(ytdlp) => tracing::info!(
                "media sites through yt-dlp {} ({}{})",
                ytdlp.version.as_deref().unwrap_or("?"),
                ytdlp.path.display(),
                if ytdlp.managed { ", managed" } else { "" }
            ),
            None => {
                tracing::warn!("yt-dlp not found: install it in Settings › Video sites to download from media sites")
            }
        }
        match &tools.ffmpeg {
            Some(ffmpeg) => tracing::info!("ffmpeg {} found", ffmpeg.version.as_deref().unwrap_or("?")),
            None => tracing::warn!(
                "ffmpeg not found: media downloads are limited to single-file formats, without conversion"
            ),
        }
        // Saved by an older version that accepted them: yt-dlp runs without any of them.
        if let Err(error) = media::options::split_extra_args(&settings.media.extra_args) {
            tracing::warn!("media.extra_args are ignored: {error}; change them in Settings › Video sites");
        }
    }

    let mut delay = UPDATE_STARTUP_DELAY;
    let mut first = true;
    loop {
        tokio::select! {
            _ = engine.shutdown_token().cancelled() => return,
            _ = tokio::time::sleep(delay) => {}
        }
        delay = UPDATE_INTERVAL;
        let settings = engine.settings.get();
        if !settings.media.enabled || !settings.media.auto_update {
            continue;
        }
        let last =
            db::kv_get(&engine.db, media::UPDATE_CHECK_KEY).await.ok().flatten().and_then(|v| v.parse::<i64>().ok());
        if !first && last.is_some_and(|last| now() - last < UPDATE_EVERY_SECS) {
            continue;
        }
        first = false;
        check_for_update(&engine, &settings).await;
    }
}

async fn check_for_update(engine: &Engine, settings: &Settings) {
    let release = match engine.media.latest_release().await {
        Ok(release) => release,
        Err(error) => {
            tracing::warn!("cannot check for yt-dlp updates: {error}");
            return;
        }
    };
    if let Err(error) = db::kv_set(&engine.db, media::UPDATE_CHECK_KEY, &now().to_string()).await {
        tracing::warn!("cannot record the yt-dlp update check: {error:#}");
    }
    match engine.media.update_managed(&settings.media, &release).await {
        Ok(Some(version)) => {
            tracing::info!("yt-dlp updated to {version}");
            engine.events.notice(
                Notice::new(NoticeLevel::Info, "media_updated", format!("yt-dlp updated to {version}"))
                    .param("version", &version),
            );
        }
        Ok(None) => tracing::debug!("yt-dlp is up to date (latest release {})", release.tag_name),
        Err(error) => tracing::warn!("cannot update yt-dlp: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_work_dirs() {
        let dir = work_dir(Path::new("downloads"), 42);
        assert!(dir.ends_with(".rustload-media-42"));
        assert!(is_work_dir(&dir));
        assert!(!is_work_dir(Path::new("downloads/movie.mkv.3.rlpart")));
    }

    #[test]
    fn explains_failures() {
        assert!(
            explain("[youtube] x: Requested format is not available. Use --list-formats", false, Origin::Link, false)
                .contains("ffmpeg")
        );
        assert_eq!(
            explain("[youtube] x: Requested format is not available. Use --list-formats", true, Origin::Link, false),
            "Requested format is not available."
        );
        assert!(explain("Unsupported URL: https://example.com/", true, Origin::WebPage, false).contains("web page"));
        assert!(
            explain("[youtube] x: Sign in to confirm your age", true, Origin::Link, false).contains("cookies file")
        );
    }

    #[test]
    fn tells_the_desktop_app_how_to_get_ffmpeg() {
        let message = "[youtube] x: Requested format is not available. Use --list-formats";
        assert!(explain(message, false, Origin::Link, false).contains("Docker image"));
        let in_app = explain(message, false, Origin::Link, true);
        assert!(!in_app.contains("Docker"));
        let expected = if media::tools::can_install_ffmpeg() { "Settings › Video sites" } else { "package manager" };
        assert!(in_app.contains(expected), "{in_app}");
    }

    #[test]
    fn maps_errors_to_retry_policy() {
        let fatal = |e: TaskError| matches!(e, TaskError::Fatal(_));
        assert!(fatal(task_error(
            MediaError::Unavailable("[youtube] x: Video unavailable".into()),
            true,
            Origin::Link,
            false
        )));
        assert!(fatal(task_error(MediaError::Missing, true, Origin::Link, false)));
        assert!(matches!(
            task_error(
                MediaError::Unavailable("HTTP Error 503: Service Unavailable".into()),
                true,
                Origin::Link,
                false
            ),
            TaskError::Transient(_)
        ));
        assert!(matches!(task_error(MediaError::Timeout(30), true, Origin::Link, false), TaskError::Transient(_)));
        assert!(matches!(task_error(MediaError::Cancelled, true, Origin::Link, false), TaskError::Stopped));
        assert!(matches!(
            task_error(MediaError::Disabled, false, Origin::Stream, false),
            TaskError::Fatal(message) if message.contains("HLS")
        ));
    }

    #[test]
    fn names_files_after_the_title() {
        let meta = |id: Option<&str>, title: Option<&str>| MediaMeta {
            id: id.map(str::to_owned),
            title: title.map(str::to_owned),
            ..MediaMeta::default()
        };
        assert_eq!(output_template(&meta(Some("abc"), Some("Clip"))), OUTPUT_TEMPLATE);
        assert_eq!(output_template(&meta(None, None)), OUTPUT_TEMPLATE);
        assert_eq!(output_template(&meta(None, Some("100% Live: a/b"))), "100%% Live_ a_b.%(ext)s");
    }

    #[test]
    fn splits_the_speed_limit() {
        assert_eq!(rate_limit(None, 2), None);
        assert_eq!(rate_limit(Some(1_024_000), 0), Some(1_024_000));
        assert_eq!(rate_limit(Some(1_024_000), 4), Some(256_000));
        assert_eq!(rate_limit(Some(2048), 4), Some(1024));
    }

    #[tokio::test]
    async fn publishes_files_with_their_subtitles() {
        let root = tempfile::tempdir().unwrap();
        let work = work_dir(root.path(), 7);
        std::fs::create_dir_all(&work).unwrap();
        std::fs::write(work.join("Clip [abc].mp4"), b"video").unwrap();
        std::fs::write(work.join("Clip [abc].en.vtt"), b"subs").unwrap();
        std::fs::write(work.join("Clip [abc].f137.mp4.part"), b"").unwrap();
        // Same video downloaded before: renamed, and its subtitles follow.
        std::fs::write(root.path().join("Clip [abc].mp4"), b"old").unwrap();

        let published = publish(&work, &work.join("Clip [abc].mp4"), root.path(), FileExists::Rename).await.unwrap();
        let Published::Moved { path, size } = published else { panic!("not moved") };
        assert_eq!(path, root.path().join("Clip [abc] (1).mp4"));
        assert_eq!(size, 5);
        assert_eq!(std::fs::read(root.path().join("Clip [abc] (1).en.vtt")).unwrap(), b"subs");
        assert!(!root.path().join("Clip [abc].f137.mp4.part").exists());

        std::fs::write(work.join("Clip [abc].mp4"), b"video").unwrap();
        let skipped = publish(&work, &work.join("Clip [abc].mp4"), root.path(), FileExists::Skip).await.unwrap();
        assert!(matches!(skipped, Published::Exists(path) if path == root.path().join("Clip [abc].mp4")));
        assert!(publish(&work, &root.path().join("Clip [abc].mp4"), root.path(), FileExists::Rename).await.is_err());
    }
}
