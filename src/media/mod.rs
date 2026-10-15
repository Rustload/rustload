//! Media sites (YouTube, Vimeo, SoundCloud, Twitch...) downloaded through yt-dlp.
//!
//! - `sites`: which links go to yt-dlp;
//! - `options`: per-package options and the yt-dlp arguments they become;
//! - `process` / `output`: the child process and its machine-readable output;
//! - `info`: what yt-dlp reports about a link;
//! - `tools`: finding yt-dlp and ffmpeg, installing and updating the managed yt-dlp.
//!
//! The queue integration (routing, link check, playlists, the download task) lives in
//! `engine/media.rs`.

pub mod info;
pub mod options;
pub mod output;
pub mod process;
pub mod sites;
pub mod tools;

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use serde::{Deserialize, Serialize};
use tokio::{sync::Semaphore, time::Instant};
use tokio_util::sync::CancellationToken;

pub use info::{MediaInfo, MediaMeta};
pub use options::{MediaOptions, MediaOptionsPatch};
pub use sites::detect_str;
pub use tools::MediaTools;

use crate::{db, engine::Engine};
use info::RawInfo;
use process::RunError;

/// Time allowed to `POST /api/media/probe` and to link checks.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(30);
/// `kv` key of the last update check (unix seconds).
pub const UPDATE_CHECK_KEY: &str = "media_update_check";

/// `media` section of the settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaSettings {
    pub enabled: bool,
    /// yt-dlp binary (or its folder); empty = managed copy, then `PATH`.
    pub ytdlp_path: String,
    /// ffmpeg binary (or its folder); empty = `<config>/bin`, then `PATH`.
    pub ffmpeg_path: String,
    /// Keep the managed yt-dlp up to date (checked daily).
    pub auto_update: bool,
    /// Try yt-dlp on web pages that no other handler recognises before failing.
    pub generic_pages: bool,
    /// Netscape `cookies.txt` for sites that need a login (relative to the config folder).
    pub cookies_file: String,
    /// Extra yt-dlp arguments, split like a shell would (no shell involved).
    pub extra_args: String,
    pub defaults: MediaOptions,
}

impl Default for MediaSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            ytdlp_path: String::new(),
            ffmpeg_path: String::new(),
            auto_update: true,
            generic_pages: false,
            cookies_file: String::new(),
            extra_args: String::new(),
            defaults: MediaOptions::default(),
        }
    }
}

impl MediaSettings {
    pub fn validate(&mut self) -> Result<(), String> {
        self.ytdlp_path = self.ytdlp_path.trim().to_owned();
        self.ffmpeg_path = self.ffmpeg_path.trim().to_owned();
        self.cookies_file = self.cookies_file.trim().to_owned();
        self.extra_args = self.extra_args.trim().to_owned();
        options::split_extra_args(&self.extra_args).map_err(|e| format!("media.extra_args: {e}"))?;
        self.defaults = self.defaults.clone().validate().map_err(|e| format!("media.defaults.{e}"))?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum MediaError {
    #[error("media sites are disabled in the settings")]
    Disabled,
    #[error("yt-dlp is not installed: install it in Settings › Video sites, or set its path")]
    Missing,
    /// yt-dlp's own explanation (private video, unsupported URL...).
    #[error("{0}")]
    Unavailable(String),
    #[error("yt-dlp did not answer within {0} s")]
    Timeout(u64),
    #[error("stopped")]
    Cancelled,
    #[error("{0}")]
    Failed(String),
}

impl From<RunError> for MediaError {
    fn from(error: RunError) -> Self {
        match error {
            RunError::Timeout(secs) => Self::Timeout(secs),
            RunError::Cancelled => Self::Cancelled,
            other => Self::Failed(other.to_string()),
        }
    }
}

/// Probes `url` with yt-dlp (`-J --flat-playlist`): title, duration, playlist entries... within
/// `timeout` (see [`YtDlp::probe`]).
pub async fn probe(engine: &Engine, url: &str, timeout: Duration) -> Result<MediaInfo, MediaError> {
    let ytdlp = YtDlp::current(engine).await?;
    ytdlp.probe(url, None, timeout, engine.shutdown_token()).await
}

/// yt-dlp ready to run with the current settings.
pub struct YtDlp {
    pub program: PathBuf,
    pub ffmpeg: Option<PathBuf>,
    js_runtimes: bool,
    cookies: Option<PathBuf>,
    cache_dir: PathBuf,
    /// Temporary folder of the runs, see `process::command`.
    pub tmp_dir: PathBuf,
    socket_timeout: u64,
    extra_args: Vec<String>,
    probes: Arc<Semaphore>,
}

impl YtDlp {
    /// `Disabled` or `Missing` when media downloads cannot run.
    pub async fn current(engine: &Engine) -> Result<Self, MediaError> {
        let settings = engine.settings.get();
        let media = &settings.media;
        if !media.enabled {
            return Err(MediaError::Disabled);
        }
        let tools = engine.media.tools(media).await;
        let program = tools.ytdlp.as_ref().ok_or(MediaError::Missing)?.path.clone();
        let config_dir = &engine.config.config_dir;
        Ok(Self {
            program,
            ffmpeg: tools.ffmpeg.as_ref().map(|t| t.path.clone()),
            js_runtimes: tools.supports_js_runtimes(),
            cookies: (!media.cookies_file.is_empty()).then(|| config_dir.join(&media.cookies_file)),
            cache_dir: config_dir.join("cache").join("yt-dlp"),
            tmp_dir: engine.media.tmp_dir(),
            socket_timeout: settings.downloads.timeout_secs,
            extra_args: options::split_extra_args(&media.extra_args).unwrap_or_default(),
            probes: engine.media.probe_slots(),
        })
    }

    pub fn has_ffmpeg(&self) -> bool {
        self.ffmpeg.is_some()
    }

    /// Arguments of every run. `cookies` is the private copy made by [`Self::cookies`].
    pub fn base_args(&self, cookies: Option<&CookieCopy>) -> Vec<OsString> {
        let mut args: Vec<OsString> =
            ["--ignore-config", "--no-playlist", "--socket-timeout"].into_iter().map(OsString::from).collect();
        args.push(self.socket_timeout.to_string().into());
        args.push("--cache-dir".into());
        args.push(self.cache_dir.clone().into());
        if let Some(ffmpeg) = &self.ffmpeg {
            args.push("--ffmpeg-location".into());
            args.push(ffmpeg.clone().into());
        }
        if self.js_runtimes {
            // Deno is enabled by default; Node.js and QuickJS (in the Docker image) are used
            // when Deno is absent. Unavailable runtimes are ignored.
            for runtime in ["node", "quickjs"] {
                args.push("--js-runtimes".into());
                args.push(runtime.into());
            }
        }
        if let Some(copy) = cookies {
            args.push("--cookies".into());
            args.push(copy.path().into());
        }
        args
    }

    /// The user's arguments, then the link.
    pub fn tail_args(&self, url: &str) -> Vec<OsString> {
        let mut args: Vec<OsString> = self.extra_args.iter().map(OsString::from).collect();
        args.push("--".into());
        args.push(url.into());
        args
    }

    /// yt-dlp writes the cookie jar back when it exits: each run gets its own copy so that
    /// parallel runs never corrupt (or rewrite) the user's file.
    pub fn cookies(&self) -> Result<Option<CookieCopy>, MediaError> {
        let Some(source) = &self.cookies else { return Ok(None) };
        if !source.is_file() {
            return Err(MediaError::Failed(format!("cookies file not found: {}", source.display())));
        }
        std::fs::create_dir_all(&self.tmp_dir).map_err(|e| MediaError::Failed(format!("cannot copy cookies: {e}")))?;
        let copy = self.tmp_dir.join(format!("cookies-{}.txt", &crate::util::random_token()[..12]));
        std::fs::copy(source, &copy).map_err(|e| MediaError::Failed(format!("cannot copy cookies: {e}")))?;
        Ok(Some(CookieCopy(copy)))
    }

    /// Runs `yt-dlp -J`. With `options`, the format selection is the download's, so the size
    /// estimate matches what will be downloaded. `timeout` includes the wait for a free slot:
    /// only a few probes run at once.
    pub async fn probe(
        &self,
        url: &str,
        options: Option<&MediaOptions>,
        timeout: Duration,
        cancel: &CancellationToken,
    ) -> Result<MediaInfo, MediaError> {
        if !(url.starts_with("http://") || url.starts_with("https://")) {
            return Err(MediaError::Unavailable("only http(s) links can be probed".into()));
        }
        let deadline = Instant::now() + timeout;
        let slot = tokio::select! {
            _ = cancel.cancelled() => return Err(MediaError::Cancelled),
            _ = tokio::time::sleep_until(deadline) => return Err(MediaError::Timeout(timeout.as_secs())),
            slot = self.probes.acquire() => slot,
        };
        // The semaphore is never closed.
        let Ok(_slot) = slot else { return Err(MediaError::Cancelled) };
        let cookies = self.cookies()?;
        let mut args = self.base_args(cookies.as_ref());
        for arg in ["-J", "--flat-playlist", "--playlist-items", "1:500", "--no-warnings"] {
            args.push(arg.into());
        }
        if let Some(options) = options {
            args.extend(options.plan(self.has_ffmpeg()).selection.into_iter().map(OsString::from));
        }
        args.extend(self.tail_args(url));
        let left = deadline.saturating_duration_since(Instant::now());
        let finished =
            process::run(&self.program, &args, &self.tmp_dir, left, cancel).await.map_err(|error| match error {
                RunError::Timeout(_) => MediaError::Timeout(timeout.as_secs()),
                other => other.into(),
            })?;
        if !finished.status.success() {
            let message = finished.error().unwrap_or_else(|| format!("yt-dlp failed ({})", finished.status));
            return Err(MediaError::Unavailable(output::clean_error(&message)));
        }
        let raw: RawInfo = serde_json::from_slice(&finished.stdout)
            .map_err(|e| MediaError::Failed(format!("unexpected yt-dlp output: {e}")))?;
        Ok(raw.into_media_info())
    }
}

/// Private copy of the cookies file, deleted after the run.
pub struct CookieCopy(PathBuf);

impl CookieCopy {
    pub fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for CookieCopy {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// `GET /api/media/status`.
#[derive(Debug, Serialize)]
pub struct MediaStatus {
    pub enabled: bool,
    pub ytdlp: Option<YtdlpStatus>,
    pub ffmpeg: Option<FfmpegStatus>,
    pub updating: bool,
    pub last_update_check: Option<i64>,
    /// Latest official release seen on GitHub (`null` until checked).
    pub latest_version: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct YtdlpStatus {
    pub path: String,
    pub version: Option<String>,
    pub managed: bool,
}

#[derive(Debug, Serialize)]
pub struct FfmpegStatus {
    pub path: String,
    pub version: Option<String>,
}

pub async fn status(engine: &Engine) -> anyhow::Result<MediaStatus> {
    let settings = engine.settings.get();
    let tools = engine.media.tools(&settings.media).await;
    let last_update_check = db::kv_get(&engine.db, UPDATE_CHECK_KEY).await?.and_then(|v| v.parse().ok());
    Ok(MediaStatus {
        enabled: settings.media.enabled,
        ytdlp: tools.ytdlp.map(|t| YtdlpStatus {
            path: t.path.to_string_lossy().into_owned(),
            version: t.version,
            managed: t.managed,
        }),
        ffmpeg: tools.ffmpeg.map(|t| FfmpegStatus { path: t.path.to_string_lossy().into_owned(), version: t.version }),
        updating: engine.media.is_updating(),
        last_update_check,
        latest_version: engine.media.latest_version(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_defaults_and_validation() {
        let json = serde_json::to_value(MediaSettings::default()).unwrap();
        assert_eq!(json["enabled"], true);
        assert_eq!(json["auto_update"], true);
        assert_eq!(json["generic_pages"], false);
        assert_eq!(json["defaults"]["quality"], "best");
        // Older configurations without the section get the defaults.
        let parsed: MediaSettings = serde_json::from_str(r#"{"generic_pages": true}"#).unwrap();
        assert!(parsed.enabled && parsed.generic_pages);

        let mut settings = MediaSettings { extra_args: " --proxy http://p:3128 ".into(), ..MediaSettings::default() };
        assert!(settings.validate().is_ok());
        assert_eq!(settings.extra_args, "--proxy http://p:3128");
        let mut settings = MediaSettings { extra_args: "-o x".into(), ..MediaSettings::default() };
        assert!(settings.validate().unwrap_err().starts_with("media.extra_args"));
        let mut settings = MediaSettings::default();
        settings.defaults.subtitle_langs = "en fr".into();
        assert!(settings.validate().unwrap_err().starts_with("media.defaults.subtitle_langs"));
    }
}
