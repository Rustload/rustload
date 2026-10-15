//! Parsing of yt-dlp's output during a download.
//!
//! Rustload never parses yt-dlp's human-readable output: it asks for machine-readable lines with
//! `--print` and `--progress-template`, each starting with a `[rustload:...]` tag, and only reads
//! `ERROR:` lines from stderr to explain failures.

use std::path::PathBuf;

use serde::Deserialize;

use super::info::{FormatSize, RawInfo};

pub const INFO_TAG: &str = "[rustload:info] ";
pub const FORMATS_TAG: &str = "[rustload:formats] ";
pub const PROGRESS_TAG: &str = "[rustload:progress] ";
pub const FILE_TAG: &str = "[rustload:file] ";

/// `--print` and `--progress-template` arguments producing the tagged lines.
pub fn output_args() -> Vec<String> {
    [
        "--newline",
        // `--print` implies `--quiet`, which hides progress unless asked for.
        "--progress",
        "--progress-template",
        "download:[rustload:progress] %(progress.{status,downloaded_bytes,total_bytes,total_bytes_estimate})j \
         %(info.format_id)s",
        "--print",
        "video:[rustload:info] %(.{id,title,ext,is_live,live_status,duration,filesize,filesize_approx,format_id,\
         extractor,uploader,channel,thumbnail,webpage_url,filename})j",
        "--print",
        "video:[rustload:formats] %(requested_formats.:.{format_id,filesize,filesize_approx})j",
        "--print",
        "after_move:[rustload:file] %(filepath)s",
    ]
    .into_iter()
    .map(str::to_owned)
    .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub enum Line {
    /// The selected video, printed before the download starts.
    Info(Box<RawInfo>),
    /// Formats downloaded separately and merged (empty for single-file formats).
    Formats(Vec<FormatSize>),
    Progress(Progress),
    /// Final path of the file, after post-processing.
    File(PathBuf),
    Error(String),
    Warning(String),
    /// Any other stderr line (usage errors are printed without the `ERROR:` prefix).
    Stderr(String),
    /// Any other stdout line.
    Other(String),
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Progress {
    pub status: String,
    pub downloaded_bytes: Option<f64>,
    pub total_bytes: Option<f64>,
    pub total_bytes_estimate: Option<f64>,
    /// Format being downloaded (one of the merged formats, or the only one).
    #[serde(skip)]
    pub format_id: String,
}

impl Progress {
    fn downloaded(&self) -> Option<u64> {
        bytes(self.downloaded_bytes)
    }

    fn total(&self) -> Option<u64> {
        bytes(self.total_bytes).or_else(|| bytes(self.total_bytes_estimate))
    }

    fn finished(&self) -> bool {
        self.status == "finished"
    }
}

fn bytes(value: Option<f64>) -> Option<u64> {
    value.filter(|v| v.is_finite() && *v >= 0.0).map(|v| v as u64)
}

pub fn parse_stdout(line: &str) -> Line {
    let line = line.trim_end_matches(['\r', '\n']);
    if let Some(json) = line.strip_prefix(INFO_TAG) {
        if let Ok(info) = serde_json::from_str::<RawInfo>(json) {
            return Line::Info(Box::new(info));
        }
    } else if let Some(json) = line.strip_prefix(FORMATS_TAG) {
        // `NA` when the field is missing (single-file format).
        return Line::Formats(serde_json::from_str(json).unwrap_or_default());
    } else if let Some(rest) = line.strip_prefix(PROGRESS_TAG) {
        let mut stream = serde_json::Deserializer::from_str(rest).into_iter::<Progress>();
        if let Some(Ok(mut progress)) = stream.next() {
            progress.format_id = rest[stream.byte_offset()..].trim().to_owned();
            return Line::Progress(progress);
        }
    } else if let Some(path) = line.strip_prefix(FILE_TAG) {
        return Line::File(PathBuf::from(path));
    }
    Line::Other(line.to_owned())
}

pub fn parse_stderr(line: &str) -> Line {
    let line = line.trim_end_matches(['\r', '\n']);
    if let Some(message) = line.strip_prefix("ERROR: ") {
        Line::Error(message.to_owned())
    } else if let Some(message) = line.strip_prefix("WARNING: ") {
        Line::Warning(message.to_owned())
    } else {
        Line::Stderr(line.to_owned())
    }
}

/// Overall progress of a download made of several phases (video then audio, plus subtitles),
/// kept monotonic so that the bar never jumps back when the next stream starts.
#[derive(Debug, Default)]
pub struct ProgressTracker {
    /// Formats announced before the download, with their expected size.
    planned: Vec<(String, Option<u64>)>,
    /// Bytes of the phases already finished.
    done: Vec<(String, u64)>,
    current: Option<Phase>,
    reported: u64,
}

#[derive(Debug)]
struct Phase {
    format_id: String,
    downloaded: u64,
    total: Option<u64>,
}

impl ProgressTracker {
    /// `floor`: bytes already shown for this download (counters never go back).
    pub fn new(floor: u64) -> Self {
        Self { reported: floor, ..Self::default() }
    }

    pub fn plan(&mut self, formats: &[FormatSize]) {
        self.planned = formats.iter().filter_map(|f| Some((f.format_id.clone()?, f.size()))).collect();
    }

    /// Plans a single-file download when no merged formats were announced.
    pub fn plan_single(&mut self, info: &RawInfo) {
        if self.planned.is_empty()
            && let Some(format_id) = &info.format_id
        {
            self.planned.push((format_id.clone(), info.size_estimate()));
        }
    }

    pub fn update(&mut self, progress: &Progress) {
        let same_phase = self.current.as_ref().is_some_and(|c| c.format_id == progress.format_id);
        if !same_phase && let Some(previous) = self.current.take() {
            self.done.push((previous.format_id, previous.downloaded));
        }
        let downloaded = progress.downloaded().or_else(|| progress.finished().then(|| progress.total()).flatten());
        let phase = self.current.get_or_insert_with(|| Phase {
            format_id: progress.format_id.clone(),
            downloaded: 0,
            total: None,
        });
        phase.downloaded = downloaded.unwrap_or(phase.downloaded);
        phase.total = progress.total().or(phase.total);
        if progress.finished()
            && let Some(finished) = self.current.take()
        {
            self.done.push((finished.format_id, finished.downloaded));
        }
        self.reported = self.reported.max(self.raw_downloaded());
    }

    fn raw_downloaded(&self) -> u64 {
        self.done.iter().map(|(_, bytes)| bytes).sum::<u64>() + self.current.as_ref().map_or(0, |c| c.downloaded)
    }

    /// Bytes downloaded so far, never decreasing.
    pub fn downloaded(&self) -> u64 {
        self.reported
    }

    /// Expected total: finished phases, the current one, and the planned formats still to come.
    /// Unknown while the current phase does not know its size.
    pub fn total(&self) -> Option<u64> {
        let started = |id: &str| {
            self.done.iter().any(|(done, _)| done == id) || self.current.as_ref().is_some_and(|c| c.format_id == id)
        };
        let upcoming = self.planned.iter().filter(|(id, _)| !started(id)).map(|(_, size)| *size);
        let current = match &self.current {
            Some(phase) => phase.total?.max(phase.downloaded),
            None => 0,
        };
        let upcoming = if self.done.is_empty() && self.current.is_none() {
            // Nothing downloaded yet: only a fully announced size is meaningful.
            upcoming.sum::<Option<u64>>().filter(|t| *t > 0)?
        } else {
            upcoming.map(|s| s.unwrap_or(0)).sum()
        };
        let done: u64 = self.done.iter().map(|(_, bytes)| bytes).sum();
        Some((done + current + upcoming).max(self.reported)).filter(|t| *t > 0)
    }
}

/// Whether a yt-dlp error is worth retrying later.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    /// The video cannot be downloaded as is (private, removed, geo-blocked, needs a login...).
    Permanent,
    /// Network problems, server errors, throttling.
    Retryable,
}

/// Sign-in and age gates: a cookies file may help.
const AUTH_PATTERNS: &[&str] = &[
    "sign in to confirm",
    "age-restricted",
    "age restricted",
    "inappropriate for some users",
    "login required",
    "log in to",
    "requires authentication",
    "only available for registered users",
    "members-only",
    "members only",
    "join this channel",
    "private video",
    "video is private",
];

/// Phrases of yt-dlp's (and the sites') permanent errors. They are matched anywhere in the
/// message, which may contain URLs: single words such as "drm" would also match retryable errors.
const PERMANENT_PATTERNS: &[&str] = &[
    "video unavailable",
    "this video is unavailable",
    "has been removed",
    "no longer available",
    "account associated with this video has been terminated",
    "does not exist",
    "http error 404",
    "http error 410",
    "available in your country",
    "geo restriction",
    "geo-restricted",
    "geo restricted",
    "unsupported url",
    "is not a valid url",
    // "This video is DRM protected"
    "is drm protected",
    // YouTube's paid videos and rentals: "This video requires payment to watch"
    "requires payment",
    "requested format is not available",
    "no video formats found",
    "live event will begin",
    "premieres in",
    "is not currently live",
    "is offline",
    // Invalid command line (extra arguments).
    "yt-dlp: error:",
];

/// Classifies a yt-dlp error message. Unknown errors are retried: the retry limit applies.
pub fn classify(message: &str) -> ErrorKind {
    let lower = message.to_ascii_lowercase();
    if AUTH_PATTERNS.iter().chain(PERMANENT_PATTERNS).any(|p| lower.contains(p)) {
        ErrorKind::Permanent
    } else {
        ErrorKind::Retryable
    }
}

/// Whether logging in (cookies) could help with this error.
pub fn needs_login(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    AUTH_PATTERNS.iter().any(|p| lower.contains(p))
}

/// The useful part of a yt-dlp error: without the `[extractor] id:` prefix and the advice to
/// report bugs or pass command-line options the user never typed.
pub fn clean_error(message: &str) -> String {
    let mut text = message.trim();
    if text.starts_with('[')
        && let Some(end) = text.find("] ")
    {
        text = &text[end + 2..];
        // `<video id>: ` follows the extractor name.
        if let Some((id, rest)) = text.split_once(": ")
            && !id.contains(' ')
        {
            text = rest;
        }
    }
    for marker in ["; please report this issue", " Use --list-formats", " Use --cookies"] {
        if let Some(index) = text.find(marker) {
            text = &text[..index];
        }
    }
    let text = text.trim().trim_end_matches([';', ',']);
    if text.is_empty() { message.trim().to_owned() } else { text.to_owned() }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn progress(status: &str, format_id: &str, downloaded: u64, total: Option<u64>) -> Progress {
        Progress {
            status: status.into(),
            downloaded_bytes: Some(downloaded as f64),
            total_bytes: total.map(|t| t as f64),
            total_bytes_estimate: None,
            format_id: format_id.into(),
        }
    }

    #[test]
    fn parses_tagged_lines() {
        let line =
            r#"[rustload:progress] {"status": "downloading", "downloaded_bytes": 1024, "total_bytes": 309288} 140"#;
        assert_eq!(parse_stdout(line), Line::Progress(progress("downloading", "140", 1024, Some(309_288))));
        let line = "[rustload:progress] {\"status\": \"downloading\", \"downloaded_bytes\": 5, \"total_bytes_estimate\": 9.5} hls-720p\r";
        let Line::Progress(p) = parse_stdout(line) else { panic!("not progress") };
        assert_eq!((p.downloaded(), p.total(), p.format_id.as_str()), (Some(5), Some(9), "hls-720p"));

        let Line::Info(info) = parse_stdout(
            r#"[rustload:info] {"id": "jNQXAC9IVRw", "title": "Me at the zoo", "ext": "m4a", "is_live": false, "filesize": 309288, "format_id": "140"}"#,
        ) else {
            panic!("not info")
        };
        assert_eq!(info.title.as_deref(), Some("Me at the zoo"));
        assert_eq!(info.size_estimate(), Some(309_288));

        assert_eq!(parse_stdout("[rustload:formats] []"), Line::Formats(vec![]));
        assert_eq!(parse_stdout("[rustload:formats] NA"), Line::Formats(vec![]));
        let Line::Formats(formats) = parse_stdout(
            r#"[rustload:formats] [{"format_id": "395", "filesize": 223779}, {"format_id": "251", "filesize_approx": 252180}]"#,
        ) else {
            panic!("not formats")
        };
        assert_eq!(formats.iter().map(FormatSize::size).collect::<Vec<_>>(), [Some(223_779), Some(252_180)]);

        assert_eq!(
            parse_stdout(r"[rustload:file] C:\dl\.rustload-media-1\Me at the zoo [jNQXAC9IVRw].m4a"),
            Line::File(PathBuf::from(r"C:\dl\.rustload-media-1\Me at the zoo [jNQXAC9IVRw].m4a"))
        );
        assert_eq!(parse_stdout("[youtube] Extracting URL"), Line::Other("[youtube] Extracting URL".into()));
        assert_eq!(
            parse_stderr("ERROR: [youtube] x: Video unavailable\r\n"),
            Line::Error("[youtube] x: Video unavailable".into())
        );
        assert_eq!(parse_stderr("WARNING: no ffmpeg"), Line::Warning("no ffmpeg".into()));
        assert_eq!(
            parse_stderr("yt-dlp: error: no such option: --nope"),
            Line::Stderr("yt-dlp: error: no such option: --nope".into())
        );
    }

    #[test]
    fn tracks_single_file_progress() {
        let mut tracker = ProgressTracker::new(0);
        tracker.plan_single(&RawInfo { format_id: Some("18".into()), filesize: Some(1000.0), ..RawInfo::default() });
        assert_eq!(tracker.total(), Some(1000));
        tracker.update(&progress("downloading", "18", 400, Some(1000)));
        assert_eq!((tracker.downloaded(), tracker.total()), (400, Some(1000)));
        tracker.update(&progress("finished", "18", 1000, Some(1000)));
        assert_eq!((tracker.downloaded(), tracker.total()), (1000, Some(1000)));
    }

    #[test]
    fn merged_progress_is_monotonic() {
        let mut tracker = ProgressTracker::new(0);
        let formats = [
            FormatSize { format_id: Some("137".into()), filesize: Some(800.0), ..FormatSize::default() },
            FormatSize { format_id: Some("140".into()), filesize_approx: Some(200.0), ..FormatSize::default() },
        ];
        tracker.plan(&formats);
        assert_eq!(tracker.total(), Some(1000));
        tracker.update(&progress("downloading", "137", 500, Some(800)));
        assert_eq!((tracker.downloaded(), tracker.total()), (500, Some(1000)));
        tracker.update(&progress("finished", "137", 800, Some(800)));
        // The audio stream starts from zero: the overall counter keeps going up.
        tracker.update(&progress("downloading", "140", 50, Some(210)));
        assert_eq!((tracker.downloaded(), tracker.total()), (850, Some(1010)));
        tracker.update(&progress("finished", "140", 210, Some(210)));
        assert_eq!((tracker.downloaded(), tracker.total()), (1010, Some(1010)));
    }

    #[test]
    fn progress_survives_missing_values() {
        // Resumed run: the counter starts where the previous one stopped.
        let mut tracker = ProgressTracker::new(700);
        tracker.update(&progress("downloading", "251", 100, None));
        assert_eq!((tracker.downloaded(), tracker.total()), (700, None));
        // A phase switch without a `finished` line (e.g. a subtitle download first).
        tracker.update(&progress("downloading", "18", 900, Some(2000)));
        assert_eq!((tracker.downloaded(), tracker.total()), (1000, Some(2100)));
        // Already downloaded: yt-dlp only reports the total.
        let mut tracker = ProgressTracker::new(0);
        tracker.update(&Progress {
            status: "finished".into(),
            total_bytes: Some(300.0),
            format_id: "140".into(),
            ..Progress::default()
        });
        assert_eq!((tracker.downloaded(), tracker.total()), (300, Some(300)));
    }

    #[test]
    fn classifies_errors() {
        use ErrorKind::*;
        for message in [
            "[youtube] abc: Private video. Sign in if you've been granted access to this video",
            "[youtube] abc: Video unavailable. This video has been removed by the uploader",
            "[youtube] abc: Sign in to confirm your age. This video may be inappropriate for some users.",
            "[youtube] abc: Sign in to confirm you’re not a bot. Use --cookies-from-browser or --cookies",
            "[youtube] abc: The uploader has not made this video available in your country",
            "Unsupported URL: https://example.com/page",
            "[youtube] abc: Requested format is not available. Use --list-formats for a list of available formats",
            "[twitch:stream] streamer: The channel is not currently live",
            "yt-dlp: error: no such option: --nope",
        ] {
            assert_eq!(classify(message), Permanent, "{message}");
        }
        for message in [
            "unable to download video data: HTTP Error 503: Service Unavailable",
            "unable to download video data: HTTP Error 429: Too Many Requests",
            "[youtube] abc: Unable to download webpage: <urlopen error [Errno 11001] getaddrinfo failed>",
            "fragment 3 not found, unable to continue",
            "HTTP Error 403: Forbidden",
            "something nobody expected",
            // Words of permanent errors inside URLs or other messages.
            "Unable to download JSON metadata: HTTP Error 502 (https://api.example.com/drm/license)",
            "[vimeo] 123: Unable to download webpage: The read operation timed out (https://vimeo.com/purchase/123)",
            "unable to download video data: <urlopen error [Errno 104] Connection reset> (rental.example.com)",
        ] {
            assert_eq!(classify(message), Retryable, "{message}");
        }
        for message in
            ["[generic] abc: This video is DRM protected", "[youtube] abc: This video requires payment to watch."]
        {
            assert_eq!(classify(message), Permanent, "{message}");
        }
        assert!(needs_login("[instagram] abc: Requested content is not available, login required"));
        assert!(!needs_login("Video unavailable"));
    }

    #[test]
    fn cleans_error_messages() {
        assert_eq!(
            clean_error("[youtube] jNQXAC9IVRw: Video unavailable. This video is private"),
            "Video unavailable. This video is private"
        );
        assert_eq!(
            clean_error(
                "[youtube] abc: Requested format is not available. Use --list-formats for a list of available formats"
            ),
            "Requested format is not available."
        );
        assert_eq!(
            clean_error(
                "[generic] x: Unable to extract title; please report this issue on https://github.com/yt-dlp/yt-dlp/issues"
            ),
            "Unable to extract title"
        );
        assert_eq!(clean_error("Unsupported URL: https://example.com/"), "Unsupported URL: https://example.com/");
        assert_eq!(
            clean_error(
                "[youtube] abc: Sign in to confirm you’re not a bot. Use --cookies-from-browser or --cookies for the authentication."
            ),
            "Sign in to confirm you’re not a bot."
        );
    }
}
