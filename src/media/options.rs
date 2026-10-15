//! Per-package media options and the yt-dlp arguments they translate to.
//!
//! Without ffmpeg yt-dlp can neither merge separate video and audio streams nor convert or tag
//! files, so the arguments degrade to single-file formats and skip post-processing; each skipped
//! feature produces a note for the log.

use serde::{Deserialize, Serialize};

/// Video height limit, or audio only.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Quality {
    #[serde(rename = "best")]
    Best,
    #[serde(rename = "2160")]
    P2160,
    #[serde(rename = "1440")]
    P1440,
    #[serde(rename = "1080")]
    P1080,
    #[serde(rename = "720")]
    P720,
    #[serde(rename = "480")]
    P480,
    #[serde(rename = "360")]
    P360,
    #[serde(rename = "audio")]
    Audio,
}

impl Quality {
    /// Resolution limit (smallest side of the picture, so that vertical videos match their label).
    fn limit(self) -> Option<u32> {
        match self {
            Self::P2160 => Some(2160),
            Self::P1440 => Some(1440),
            Self::P1080 => Some(1080),
            Self::P720 => Some(720),
            Self::P480 => Some(480),
            Self::P360 => Some(360),
            Self::Best | Self::Audio => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AudioFormat {
    Best,
    M4a,
    Mp3,
    Opus,
    Flac,
}

impl AudioFormat {
    fn name(self) -> &'static str {
        match self {
            Self::Best => "best",
            Self::M4a => "m4a",
            Self::Mp3 => "mp3",
            Self::Opus => "opus",
            Self::Flac => "flac",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Container {
    Auto,
    Mp4,
    Mkv,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SponsorBlock {
    Off,
    /// Sponsored segments become chapters.
    Mark,
    /// Sponsored segments are cut out (YouTube only).
    Remove,
}

/// What to download from a media link and how to post-process it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaOptions {
    pub quality: Quality,
    /// Used when `quality` is `audio`.
    pub audio_format: AudioFormat,
    pub container: Container,
    pub subtitles: bool,
    /// Comma-separated languages (`en,fr`, regular expressions such as `en.*`, or `all`).
    pub subtitle_langs: String,
    pub embed_metadata: bool,
    pub embed_thumbnail: bool,
    pub sponsorblock: SponsorBlock,
}

impl Default for MediaOptions {
    fn default() -> Self {
        Self {
            quality: Quality::Best,
            audio_format: AudioFormat::Best,
            container: Container::Auto,
            subtitles: false,
            subtitle_langs: "en".into(),
            embed_metadata: true,
            embed_thumbnail: true,
            sponsorblock: SponsorBlock::Off,
        }
    }
}

/// Partial options from the API: missing fields keep the base values (settings defaults when
/// adding, current values when patching a package).
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct MediaOptionsPatch {
    pub quality: Option<Quality>,
    pub audio_format: Option<AudioFormat>,
    pub container: Option<Container>,
    pub subtitles: Option<bool>,
    pub subtitle_langs: Option<String>,
    pub embed_metadata: Option<bool>,
    pub embed_thumbnail: Option<bool>,
    pub sponsorblock: Option<SponsorBlock>,
}

impl MediaOptionsPatch {
    pub fn apply(&self, base: &MediaOptions) -> MediaOptions {
        MediaOptions {
            quality: self.quality.unwrap_or(base.quality),
            audio_format: self.audio_format.unwrap_or(base.audio_format),
            container: self.container.unwrap_or(base.container),
            subtitles: self.subtitles.unwrap_or(base.subtitles),
            subtitle_langs: self.subtitle_langs.clone().unwrap_or_else(|| base.subtitle_langs.clone()),
            embed_metadata: self.embed_metadata.unwrap_or(base.embed_metadata),
            embed_thumbnail: self.embed_thumbnail.unwrap_or(base.embed_thumbnail),
            sponsorblock: self.sponsorblock.unwrap_or(base.sponsorblock),
        }
    }
}

impl From<MediaOptions> for MediaOptionsPatch {
    fn from(options: MediaOptions) -> Self {
        Self {
            quality: Some(options.quality),
            audio_format: Some(options.audio_format),
            container: Some(options.container),
            subtitles: Some(options.subtitles),
            subtitle_langs: Some(options.subtitle_langs),
            embed_metadata: Some(options.embed_metadata),
            embed_thumbnail: Some(options.embed_thumbnail),
            sponsorblock: Some(options.sponsorblock),
        }
    }
}

impl MediaOptions {
    /// Normalises the subtitle languages and rejects values yt-dlp would misread.
    pub fn validate(mut self) -> Result<Self, String> {
        let langs: Vec<&str> = self.subtitle_langs.split(',').map(str::trim).filter(|l| !l.is_empty()).collect();
        let valid = |lang: &&str| lang.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '*'));
        if self.subtitle_langs.len() > 200 || !langs.iter().all(valid) {
            return Err(
                "subtitle_langs must be a comma-separated list of language codes such as `en,fr` or `all`".into()
            );
        }
        self.subtitle_langs = if langs.is_empty() { "en".into() } else { langs.join(",") };
        Ok(self)
    }

    pub fn is_audio(&self) -> bool {
        self.quality == Quality::Audio
    }

    /// Extension the file will most likely have, used to name links before they are downloaded
    /// (and to pick the category automatically). `fallback` is what yt-dlp announced, if known.
    pub fn expected_extension(&self, ffmpeg: bool, fallback: Option<&str>) -> String {
        let known = match (self.is_audio(), ffmpeg) {
            (true, true) if self.audio_format != AudioFormat::Best => Some(self.audio_format.name()),
            // Audio extracted from a WebM file keeps its codec: Opus.
            (true, true) if fallback == Some("webm") => Some("opus"),
            (true, false) if self.audio_format == AudioFormat::M4a => Some("m4a"),
            (false, true) => match self.container {
                Container::Mp4 => Some("mp4"),
                Container::Mkv => Some("mkv"),
                Container::Auto => None,
            },
            _ => None,
        };
        let default = if self.is_audio() { "m4a" } else { "mp4" };
        known.or(fallback.filter(|e| !e.is_empty())).unwrap_or(default).to_owned()
    }

    /// Format selection and post-processing arguments for yt-dlp.
    pub fn plan(&self, ffmpeg: bool) -> FormatPlan {
        let mut plan = FormatPlan::default();
        if self.is_audio() {
            self.plan_audio(ffmpeg, &mut plan);
        } else {
            self.plan_video(ffmpeg, &mut plan);
        }

        if self.embed_metadata || self.embed_thumbnail {
            if ffmpeg {
                if self.embed_metadata {
                    plan.push(["--embed-metadata"]);
                }
                if self.embed_thumbnail {
                    plan.push(["--embed-thumbnail"]);
                }
            } else {
                plan.notes.push("metadata and cover embedding skipped (needs ffmpeg)");
            }
        }
        match (self.sponsorblock, ffmpeg) {
            (SponsorBlock::Off, _) => {}
            (SponsorBlock::Mark, true) => plan.push(["--sponsorblock-mark", "all", "--embed-chapters"]),
            (SponsorBlock::Remove, true) => plan.push(["--sponsorblock-remove", "default"]),
            (_, false) => plan.notes.push("SponsorBlock skipped (needs ffmpeg)"),
        }
        plan
    }

    fn plan_video(&self, ffmpeg: bool, plan: &mut FormatPlan) {
        let mut sort: Vec<String> = Vec::new();
        if let Some(limit) = self.quality.limit() {
            // `res:N` prefers the best picture not above N, and the smallest one above N when
            // nothing smaller exists, instead of failing.
            sort.push(format!("res:{limit}"));
        }
        if self.container == Container::Mp4 {
            // Widely playable codecs first: H.264 video and AAC audio.
            sort.push("+codec:avc:m4a".into());
        }
        if ffmpeg {
            plan.select(["-f", "bv*+ba/b"]);
            match self.container {
                Container::Auto => {}
                Container::Mp4 => plan.select(["--merge-output-format", "mp4"]),
                Container::Mkv => plan.select(["--merge-output-format", "mkv", "--remux-video", "mkv"]),
            }
        } else {
            plan.select(["-f", "b"]);
            plan.notes.push("ffmpeg not found: single-file formats only (often limited to 360p or 720p on YouTube)");
            if self.container == Container::Mkv {
                plan.notes.push("mkv container skipped (needs ffmpeg)");
            }
        }
        if !sort.is_empty() {
            plan.select(["-S".to_owned(), sort.join(",")]);
        }
        if self.subtitles {
            plan.push(["--write-subs", "--sub-langs", &self.subtitle_langs]);
            if ffmpeg {
                plan.push(["--embed-subs"]);
            } else {
                plan.notes.push("subtitles saved as separate files (embedding needs ffmpeg)");
            }
        }
    }

    fn plan_audio(&self, ffmpeg: bool, plan: &mut FormatPlan) {
        let selector = match self.audio_format {
            AudioFormat::M4a => "ba[ext=m4a]/ba/b",
            AudioFormat::Opus => "ba[acodec^=opus]/ba/b",
            // Without conversion, M4A is the most widely playable original (the best audio
            // stream is often Opus in a WebM file, which players and categories take for a video).
            _ if !ffmpeg => "ba[ext=m4a]/ba/b",
            _ => "ba/b",
        };
        plan.select(["-f", selector]);
        if ffmpeg {
            plan.push(["-x", "--audio-format", self.audio_format.name()]);
            if matches!(self.audio_format, AudioFormat::M4a | AudioFormat::Mp3 | AudioFormat::Opus) {
                // Only applies when converting: keep transcoding losses as low as possible.
                plan.push(["--audio-quality", "0"]);
            }
        } else if matches!(self.audio_format, AudioFormat::Mp3 | AudioFormat::Flac) {
            plan.notes.push("audio conversion skipped (needs ffmpeg): the original audio format is kept");
        }
    }
}

/// yt-dlp arguments for a download, and what had to be left out.
#[derive(Debug, Default, PartialEq)]
pub struct FormatPlan {
    /// Format selection: also given to probes, so that size estimates match the download.
    pub selection: Vec<String>,
    /// Post-processing (conversion, embedding, SponsorBlock...).
    pub processing: Vec<String>,
    pub notes: Vec<&'static str>,
}

impl FormatPlan {
    fn select<I: IntoIterator<Item = S>, S: Into<String>>(&mut self, args: I) {
        self.selection.extend(args.into_iter().map(Into::into));
    }

    fn push<I: IntoIterator<Item = S>, S: Into<String>>(&mut self, args: I) {
        self.processing.extend(args.into_iter().map(Into::into));
    }

    pub fn args(&self) -> impl Iterator<Item = &String> {
        self.selection.iter().chain(&self.processing)
    }
}

/// Options Rustload sets itself: overriding them would break progress tracking or file
/// handling. yt-dlp takes any unambiguous beginning of a long option for the option itself
/// (`--pat` for `--paths`): beginnings of these names are refused too.
const RESERVED_LONG: &[&str] = &[
    "--output",
    "--paths",
    "--print",
    "--print-to-file",
    "--progress-template",
    "--newline",
    "--no-progress",
    "--progress",
    "--quiet",
    "--simulate",
    "--skip-download",
    "--no-download",
    "--dump-json",
    "--dump-single-json",
    "--batch-file",
    "--config-locations",
    "--load-info-json",
    "--update",
    "--update-to",
    "--yes-playlist",
    "--keep-video",
    "--split-chapters",
    "--list-formats",
    "--list-subs",
    "--list-thumbnails",
    "--version",
    "--help",
    // Set from `ffmpeg_path`; another value would run any program.
    "--ffmpeg-location",
];
const RESERVED_SHORT: &[char] = &['o', 'P', 'O', 'q', 's', 'j', 'J', 'a', 'U', 'k', 'F', 'h'];
/// Options that run commands or load code (commands belong in the hooks), refused like
/// [`RESERVED_LONG`], plus every option starting with [`EXEC`].
const UNSAFE_LONG: &[&str] =
    &["--exec", "--exec-before-download", "--netrc-cmd", "--use-postprocessor", "--plugin-dirs"];
const EXEC: &str = "--exec";
/// Complete options that are also the beginning of a refused one.
const ALLOWED_LONG: &[&str] = &["--netrc"];

/// Splits user-provided extra arguments like a shell would (quotes group words; inside double
/// quotes `\"` and `\\` are escapes) without ever running a shell, and rejects options that
/// Rustload manages.
pub fn split_extra_args(raw: &str) -> Result<Vec<String>, String> {
    let mut args = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut chars = raw.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(c) => current.push(c),
                        None => return Err("unbalanced single quote".into()),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') if matches!(chars.peek(), Some('"' | '\\')) => {
                            current.extend(chars.next());
                        }
                        Some(c) => current.push(c),
                        None => return Err("unbalanced double quote".into()),
                    }
                }
            }
            c if c.is_whitespace() => {
                if in_word {
                    args.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            c => {
                in_word = true;
                current.push(c);
            }
        }
    }
    if in_word {
        args.push(current);
    }
    match args.iter().find_map(|arg| refusal(arg)) {
        Some(reason) => Err(reason),
        None => Ok(args),
    }
}

/// Why `arg` cannot be given to yt-dlp, if it cannot.
fn refusal(arg: &str) -> Option<String> {
    const MANAGED: &str = "is managed by Rustload and cannot be overridden";
    if let Some(long) = arg.strip_prefix("--") {
        let option = format!("--{}", long.split('=').next().unwrap_or_default());
        if option == "--" {
            return Some(format!("`--` {MANAGED} (the link comes after it)"));
        }
        if ALLOWED_LONG.contains(&option.as_str()) {
            return None;
        }
        let target = |list: &[&'static str]| list.iter().copied().find(|full| full.starts_with(&option));
        let named = |full: &str| {
            if full == option { format!("`{option}`") } else { format!("`{option}` (beginning of `{full}`)") }
        };
        if let Some(full) = target(UNSAFE_LONG) {
            return Some(format!("{} is not allowed: it runs commands or loads code", named(full)));
        }
        if option.starts_with(EXEC) {
            return Some(format!("`{option}` is not allowed: it runs commands or loads code"));
        }
        return target(RESERVED_LONG).map(|full| format!("{} {MANAGED}", named(full)));
    }
    // A single-dash option, possibly with its value attached (`-ofile`) or combined (`-qs`).
    let short = arg.strip_prefix('-')?.chars().next()?;
    RESERVED_SHORT.contains(&short).then(|| format!("`{arg}` {MANAGED}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options(quality: Quality) -> MediaOptions {
        MediaOptions { quality, embed_metadata: false, embed_thumbnail: false, ..MediaOptions::default() }
    }

    fn args(plan: &FormatPlan) -> String {
        plan.args().cloned().collect::<Vec<_>>().join(" ")
    }

    #[test]
    fn options_serde_and_defaults() {
        let defaults = MediaOptions::default();
        let json = serde_json::to_value(&defaults).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "quality": "best", "audio_format": "best", "container": "auto", "subtitles": false,
                "subtitle_langs": "en", "embed_metadata": true, "embed_thumbnail": true, "sponsorblock": "off"
            })
        );
        let parsed: MediaOptions = serde_json::from_str(r#"{"quality":"1080","sponsorblock":"remove"}"#).unwrap();
        assert_eq!(parsed.quality, Quality::P1080);
        assert_eq!(parsed.sponsorblock, SponsorBlock::Remove);
        assert_eq!(parsed.container, Container::Auto);
        assert!(serde_json::from_str::<MediaOptions>(r#"{"quality":"999"}"#).is_err());
    }

    #[test]
    fn patches_apply_over_a_base() {
        let base = MediaOptions { subtitles: true, ..MediaOptions::default() };
        let patch: MediaOptionsPatch = serde_json::from_str(r#"{"quality":"audio","audio_format":"mp3"}"#).unwrap();
        let applied = patch.apply(&base);
        assert_eq!(applied.quality, Quality::Audio);
        assert_eq!(applied.audio_format, AudioFormat::Mp3);
        assert!(applied.subtitles);
        assert_eq!(MediaOptionsPatch::from(applied.clone()).apply(&MediaOptions::default()), applied);
    }

    #[test]
    fn validates_subtitle_languages() {
        let with = |langs: &str| MediaOptions { subtitle_langs: langs.into(), ..MediaOptions::default() }.validate();
        assert_eq!(with(" en , fr ").unwrap().subtitle_langs, "en,fr");
        assert_eq!(with("all,-live_chat").unwrap().subtitle_langs, "all,-live_chat");
        assert_eq!(with("en.*,pt-BR").unwrap().subtitle_langs, "en.*,pt-BR");
        assert_eq!(with("").unwrap().subtitle_langs, "en");
        assert!(with("en;rm -rf").is_err());
        assert!(with("en fr").is_err());
    }

    #[test]
    fn builds_video_selectors() {
        assert_eq!(args(&options(Quality::Best).plan(true)), "-f bv*+ba/b");
        assert_eq!(args(&options(Quality::P1080).plan(true)), "-f bv*+ba/b -S res:1080");
        let mp4 = MediaOptions { container: Container::Mp4, ..options(Quality::P720) };
        assert_eq!(args(&mp4.plan(true)), "-f bv*+ba/b --merge-output-format mp4 -S res:720,+codec:avc:m4a");
        let mkv = MediaOptions { container: Container::Mkv, ..options(Quality::Best) };
        assert_eq!(args(&mkv.plan(true)), "-f bv*+ba/b --merge-output-format mkv --remux-video mkv");
    }

    #[test]
    fn degrades_video_without_ffmpeg() {
        let plan = options(Quality::P1080).plan(false);
        assert_eq!(args(&plan), "-f b -S res:1080");
        assert_eq!(plan.notes.len(), 1);
        let rich = MediaOptions {
            subtitles: true,
            subtitle_langs: "en,fr".into(),
            sponsorblock: SponsorBlock::Remove,
            ..MediaOptions::default()
        };
        let plan = rich.plan(false);
        assert_eq!(args(&plan), "-f b --write-subs --sub-langs en,fr");
        assert!(plan.notes.iter().any(|n| n.contains("subtitles")));
        assert!(plan.notes.iter().any(|n| n.contains("SponsorBlock")));
        assert!(plan.notes.iter().any(|n| n.contains("metadata")));
        assert_eq!(
            args(&rich.plan(true)),
            "-f bv*+ba/b --write-subs --sub-langs en,fr --embed-subs --embed-metadata --embed-thumbnail \
             --sponsorblock-remove default"
        );
    }

    #[test]
    fn builds_audio_selectors() {
        let audio = |format: AudioFormat| MediaOptions { audio_format: format, ..options(Quality::Audio) };
        assert_eq!(args(&audio(AudioFormat::Best).plan(true)), "-f ba/b -x --audio-format best");
        assert_eq!(args(&audio(AudioFormat::Mp3).plan(true)), "-f ba/b -x --audio-format mp3 --audio-quality 0");
        assert_eq!(
            args(&audio(AudioFormat::M4a).plan(true)),
            "-f ba[ext=m4a]/ba/b -x --audio-format m4a --audio-quality 0"
        );
        assert_eq!(args(&audio(AudioFormat::Flac).plan(true)), "-f ba/b -x --audio-format flac");
        assert_eq!(args(&audio(AudioFormat::Best).plan(false)), "-f ba[ext=m4a]/ba/b");
        let mp3 = audio(AudioFormat::Mp3).plan(false);
        assert_eq!(args(&mp3), "-f ba[ext=m4a]/ba/b");
        assert_eq!(mp3.notes.len(), 1);
        // Subtitles only make sense for videos.
        let with_subs = MediaOptions { subtitles: true, ..audio(AudioFormat::Opus) };
        assert_eq!(args(&with_subs.plan(true)), "-f ba[acodec^=opus]/ba/b -x --audio-format opus --audio-quality 0");
    }

    #[test]
    fn predicts_extensions() {
        let audio = |format: AudioFormat| MediaOptions { audio_format: format, ..options(Quality::Audio) };
        assert_eq!(audio(AudioFormat::Mp3).expected_extension(true, Some("webm")), "mp3");
        assert_eq!(audio(AudioFormat::Mp3).expected_extension(false, Some("m4a")), "m4a");
        assert_eq!(audio(AudioFormat::Best).expected_extension(true, None), "m4a");
        assert_eq!(audio(AudioFormat::Best).expected_extension(true, Some("webm")), "opus");
        assert_eq!(audio(AudioFormat::Best).expected_extension(false, Some("webm")), "webm");
        let mkv = MediaOptions { container: Container::Mkv, ..options(Quality::Best) };
        assert_eq!(mkv.expected_extension(true, Some("webm")), "mkv");
        assert_eq!(mkv.expected_extension(false, Some("webm")), "webm");
        assert_eq!(options(Quality::Best).expected_extension(true, None), "mp4");
    }

    #[test]
    fn splits_extra_arguments() {
        assert_eq!(
            split_extra_args(r#"--proxy socks5://127.0.0.1:1080 --referer "https://a.b/c d" -N 4"#).unwrap(),
            ["--proxy", "socks5://127.0.0.1:1080", "--referer", "https://a.b/c d", "-N", "4"]
        );
        assert_eq!(
            split_extra_args(r#"--ppa 'Merger:-v quiet' "a\"b" ''"#).unwrap(),
            ["--ppa", "Merger:-v quiet", r#"a"b"#, ""]
        );
        assert_eq!(split_extra_args(r"--cookies C:\Users\me\c.txt").unwrap(), ["--cookies", r"C:\Users\me\c.txt"]);
        assert!(split_extra_args("   ").unwrap().is_empty());
        assert!(split_extra_args(r#"--referer "unterminated"#).is_err());
        assert!(split_extra_args("-o /tmp/x").is_err());
        assert!(split_extra_args("-o/tmp/x").is_err());
        assert!(split_extra_args("--output=x").is_err());
        assert!(split_extra_args("--exec 'rm -rf /'").is_err());
        assert!(split_extra_args("-qs").is_err());
        assert!(split_extra_args("--print-traffic").is_ok());
        assert!(split_extra_args("--sub-format srt").is_ok());
    }

    #[test]
    fn refuses_abbreviations_and_unsafe_options() {
        let refused = |args: &str| split_extra_args(args).unwrap_err();
        // yt-dlp expands unambiguous beginnings of long options.
        assert_eq!(
            refused("--pat /tmp"),
            "`--pat` (beginning of `--paths`) is managed by Rustload and cannot be overridden"
        );
        assert_eq!(refused("--paths=/tmp"), "`--paths` is managed by Rustload and cannot be overridden");
        for args in [
            "--pa=/tmp",
            "--load-info x.json",
            "--load-info-json x.json",
            "--config-location x.conf",
            "--config-locations x.conf",
            "--ffmpeg-location /bin/sh",
            "--no-prog",
            "-- https://example.com/other",
        ] {
            assert!(split_extra_args(args).is_err(), "{args}");
        }
        assert_eq!(refused("--netrc-cmd 'cat x'"), "`--netrc-cmd` is not allowed: it runs commands or loads code");
        assert_eq!(
            refused("--use-post X"),
            "`--use-post` (beginning of `--use-postprocessor`) is not allowed: it runs commands or loads code"
        );
        for args in [
            "--exec-before-download id",
            "--exec-b x",
            "--exec=x",
            "--execute-anything",
            "--plugin-dirs ./p",
            "--netrc-c x",
        ] {
            assert!(split_extra_args(args).unwrap_err().contains("runs commands"), "{args}");
        }
        // Complete options, even when they begin a refused one.
        for args in ["--netrc", "--netrc-location ~/.netrc", "--no-exec", "--proxy http://p:3128", "--progress-delta 2"]
        {
            assert!(split_extra_args(args).is_ok(), "{args}");
        }
    }
}
