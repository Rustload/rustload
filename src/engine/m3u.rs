//! M3U playlists (`.m3u8`, `.m3u`): Rustload reads them to know what they are. An HLS stream
//! (`#EXT-X-` tags) is one video, which yt-dlp downloads and assembles. Any other playlist is a
//! list of links, added as its entries like the files of a folder.
//!
//! Links ending in `.m3u8`/`.m3u` are read when checked and when added; links recognised by their
//! `Content-Type` only are read by the link check, the availability check and, as a last resort,
//! when their download starts (HLS streams only: a list found then is downloaded as a file).

use std::{collections::HashSet, time::Duration};

use reqwest::header;
use url::Url;

use super::{Engine, naming};
use crate::{
    db::{self, downloads::NewDownload, packages::PackageUpdate},
    media::{MediaMeta, info::title_from_url, sites::Site},
    models::DownloadRow,
    util::url_host,
};

/// Bytes of a playlist read to classify it and list its links: HLS tags come first.
const MAX_BYTES: usize = 1024 * 1024;
/// Links kept from a list.
pub const MAX_ENTRIES: usize = 1000;
const FETCH_TIMEOUT: Duration = Duration::from_secs(20);
/// Media types of M3U playlists, HLS and plain alike.
const CONTENT_TYPES: [&str; 5] = [
    "application/vnd.apple.mpegurl",
    "application/mpegurl",
    "application/x-mpegurl",
    "audio/mpegurl",
    "audio/x-mpegurl",
];

/// HLS streams are shown and routed like a media site downloaded by yt-dlp.
pub const STREAM: Site = Site { id: "hls", name: "HLS", collection: false };
pub const NEEDS_YTDLP: &str = "HLS streams are downloaded with yt-dlp: enable video sites in Settings › Video sites";

#[derive(Debug, PartialEq)]
pub enum Playlist {
    /// An HLS master or media playlist.
    Stream,
    /// A list of links, possibly empty.
    List(Vec<Entry>),
}

#[derive(Debug, PartialEq)]
pub struct Entry {
    pub url: String,
    /// From the `#EXTINF` line before the link.
    pub title: Option<String>,
}

/// Whether the path of `url` ends in `.m3u8` or `.m3u` (the query is not part of it).
pub fn is_candidate_url(url: &str) -> bool {
    naming::from_url(url).is_some_and(|name| has_playlist_extension(&name))
}

pub fn has_playlist_extension(name: &str) -> bool {
    matches!(naming::extension(name).as_deref(), Some("m3u8" | "m3u"))
}

/// Whether a `Content-Type` header announces an M3U playlist.
pub fn is_playlist_type(content_type: &str) -> bool {
    let essence = content_type.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    CONTENT_TYPES.contains(&essence.as_str())
}

/// Package name of a list, title of a stream: see [`title_from_url`].
pub fn list_name(url: &str) -> Option<String> {
    title_from_url(url)
}

/// What a stream download carries until yt-dlp tells more: it routes the download to yt-dlp
/// (`engine::media::routes`) and names it.
pub fn stream_meta(url: &str) -> MediaMeta {
    MediaMeta { title: title_from_url(url), ..MediaMeta::default() }
}

/// Reads the playlist at `url`. `None` when it is not an M3U playlist (a web page, an error
/// message...) or cannot be read: the link then follows its usual route. A `.m3u8` link that
/// does not answer Rustload is taken for a stream: HLS servers often only answer the player of
/// their site, and yt-dlp can send what it expects (`--referer` in `media.extra_args`).
pub async fn fetch(client: &reqwest::Client, url: &str) -> Option<Playlist> {
    let request = client.get(url).timeout(FETCH_TIMEOUT).send().await.and_then(|r| r.error_for_status());
    let mut response = match request {
        Ok(response) => response,
        Err(error) => {
            tracing::debug!("cannot read playlist: {}", error.without_url());
            let hls = naming::from_url(url).and_then(|name| naming::extension(&name)).as_deref() == Some("m3u8");
            return hls.then_some(Playlist::Stream);
        }
    };
    let html = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value.to_ascii_lowercase().starts_with("text/html"));
    if html {
        return None;
    }
    let base = response.url().clone();
    let mut body = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                body.extend_from_slice(&chunk[..chunk.len().min(MAX_BYTES - body.len())]);
                if body.len() == MAX_BYTES {
                    // Only whole lines count.
                    body.truncate(body.iter().rposition(|&b| b == b'\n').unwrap_or(0));
                    break;
                }
            }
            Ok(None) => break,
            Err(error) => {
                tracing::debug!("cannot read playlist: {}", error.without_url());
                return None;
            }
        }
    }
    parse(&String::from_utf8_lossy(&body), &base)
}

/// Classifies a playlist; `base` resolves relative links. `None` for anything that is not an M3U
/// playlist: markup, or text without the `#EXTM3U` header that is not only absolute links.
pub fn parse(text: &str, base: &Url) -> Option<Playlist> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    if text.trim_start().starts_with('<') {
        return None;
    }
    let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty()).peekable();
    let header = lines.peek().is_some_and(|line| line.starts_with("#EXTM3U"));
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    let mut title = None;
    for line in lines {
        if line.starts_with("#EXT-X-") {
            return Some(Playlist::Stream);
        }
        if let Some(info) = line.strip_prefix("#EXTINF:") {
            title = extinf_title(info);
            continue;
        }
        if line.starts_with('#') {
            continue;
        }
        // Relative links only make sense in a real playlist: without the header, a line of
        // plain text would become a link.
        let url = if header { base.join(line).ok() } else { Url::parse(line).ok() };
        match url.filter(|url| matches!(url.scheme(), "http" | "https")) {
            Some(url) => {
                let url = String::from(url);
                let title = title.take();
                if entries.len() < MAX_ENTRIES && seen.insert(url.clone()) {
                    entries.push(Entry { url, title });
                }
            }
            // Local paths and other protocols (`rtsp://`) cannot be downloaded.
            None if header => title = None,
            None => return None,
        }
    }
    (header || !entries.is_empty()).then_some(Playlist::List(entries))
}

/// `-1 tvg-id="a,b" group-title="News",France 24` → `France 24`: what follows the first comma
/// outside quotes.
fn extinf_title(info: &str) -> Option<String> {
    let mut quoted = false;
    for (index, c) in info.char_indices() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => {
                let title = info[index + 1..].trim();
                return (!title.is_empty()).then(|| title.to_owned());
            }
            _ => {}
        }
    }
    None
}

/// A queued link that the availability check found to be a playlist although it was added as a
/// file: a stream goes to yt-dlp, a list is replaced by its links. Returns whether it was one.
pub(super) async fn settle_queued(engine: &Engine, download: &DownloadRow) -> bool {
    let Some(playlist) = fetch(&engine.download_client(), &download.url).await else { return false };
    let result = match playlist {
        Playlist::Stream => settle_stream(engine, download).await,
        Playlist::List(entries) if !entries.is_empty() => settle_list(engine, download, entries).await,
        // Nothing to add instead: the file is downloaded as it is.
        Playlist::List(_) => return false,
    };
    if let Err(error) = result {
        tracing::warn!("cannot store link check: {error:#}");
    }
    true
}

async fn settle_stream(engine: &Engine, download: &DownloadRow) -> anyhow::Result<()> {
    tracing::info!("{} is an HLS stream: yt-dlp downloads it", download.url);
    db::media::set_download_meta(&engine.db, download.id, &stream_meta(&download.url)).await?;
    // yt-dlp tells its title, duration and size when it can.
    match db::downloads::get(&engine.db, download.id).await? {
        Some(row) if super::media::check(engine, &row).await => Ok(()),
        _ => db::downloads::record_check(&engine.db, download.id, None, None, url_host(&download.url).as_deref()).await,
    }
}

async fn settle_list(engine: &Engine, download: &DownloadRow, entries: Vec<Entry>) -> anyhow::Result<()> {
    let links: Vec<NewDownload> = entries.into_iter().map(|entry| NewDownload::link(entry.url)).collect();
    if !db::downloads::replace_with_links(&engine.db, download.id, download.package_id, &links).await? {
        return Ok(());
    }
    tracing::info!("{} listed: {} link(s)", download.url, links.len());
    if let (Some(name), Some(package)) =
        (list_name(&download.url), db::packages::get(&engine.db, download.package_id).await?)
        && !package.name_locked
        && package.dest_dir.is_none()
    {
        let update = PackageUpdate { name: Some(name), name_locked: Some(true), ..Default::default() };
        db::packages::update(&engine.db, package.id, update).await?;
    }
    engine.events.queue_changed();
    engine.wake();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> Url {
        Url::parse("https://cdn.example.com/music/lists/album.m3u").unwrap()
    }

    fn list(text: &str) -> Vec<(String, Option<String>)> {
        match parse(text, &base()) {
            Some(Playlist::List(entries)) => entries.into_iter().map(|e| (e.url, e.title)).collect(),
            other => panic!("not a list: {other:?}"),
        }
    }

    #[test]
    fn recognises_hls_playlists() {
        let master = "#EXTM3U\n#EXT-X-STREAM-INF:BANDWIDTH=1280000,RESOLUTION=640x360\nlow/index.m3u8\n\
                      #EXT-X-STREAM-INF:BANDWIDTH=2560000,RESOLUTION=1280x720\nhigh/index.m3u8\n";
        assert_eq!(parse(master, &base()), Some(Playlist::Stream));
        let media = "\u{feff}#EXTM3U\r\n#EXT-X-VERSION:3\r\n#EXT-X-TARGETDURATION:10\r\n#EXTINF:9.97,\r\n\
                     segment0.ts\r\n#EXTINF:9.97,\r\nsegment1.ts\r\n#EXT-X-ENDLIST\r\n";
        assert_eq!(parse(media, &base()), Some(Playlist::Stream));
    }

    #[test]
    fn lists_links_with_their_titles() {
        let text = "\u{feff}#EXTM3U\r\n#EXTINF:215,Artist - First song\r\n01 First song.mp3\r\n\
                    #EXTINF:-1 tvg-id=\"a,b\" group-title=\"News\",France 24\r\n\
                    https://other.example.org/live/stream.m3u8\r\n# a comment\r\n../covers/front.jpg\r\n\
                    01 First song.mp3\r\n#EXTINF:1,Local file\r\nC:\\Music\\song.mp3\r\nrtsp://camera/feed\r\n\
                    https://cdn.example.com/music/after-a-skipped-title.mp3\r\n";
        assert_eq!(
            list(text),
            [
                (
                    "https://cdn.example.com/music/lists/01%20First%20song.mp3".into(),
                    Some("Artist - First song".into())
                ),
                ("https://other.example.org/live/stream.m3u8".into(), Some("France 24".into())),
                ("https://cdn.example.com/music/covers/front.jpg".into(), None),
                ("https://cdn.example.com/music/after-a-skipped-title.mp3".into(), None),
            ]
        );
    }

    #[test]
    fn accepts_lists_of_absolute_links_without_header() {
        assert_eq!(
            list("https://a.example.com/1.flac\nhttps://a.example.com/2.flac\n"),
            [("https://a.example.com/1.flac".into(), None), ("https://a.example.com/2.flac".into(), None)]
        );
    }

    #[test]
    fn rejects_what_is_not_a_playlist() {
        assert_eq!(parse("#EXTM3U\n# nothing here\n", &base()), Some(Playlist::List(Vec::new())));
        assert_eq!(parse("", &base()), None);
        assert_eq!(parse("  <!DOCTYPE html><html><body>Login</body></html>", &base()), None);
        assert_eq!(parse("Access denied\n", &base()), None);
        assert_eq!(parse("https://a.example.com/1.flac\nsong.mp3\n", &base()), None);
    }

    #[test]
    fn caps_and_deduplicates_lists() {
        let text: String =
            std::iter::once("#EXTM3U\n".to_owned()).chain((0..1200).map(|i| format!("{}.mp3\n", i % 1100))).collect();
        let entries = list(&text);
        assert_eq!(entries.len(), MAX_ENTRIES);
        assert_eq!(entries[999].0, "https://cdn.example.com/music/lists/999.mp3");
    }

    #[test]
    fn recognises_candidates() {
        assert!(is_candidate_url("https://cdn.example.com/x36xhzz/x36xhzz.m3u8?token=a.b"));
        assert!(is_candidate_url("https://radio.example.com/LIST.M3U"));
        assert!(!is_candidate_url("https://cdn.example.com/m3u8/video.mp4"));
        assert!(!is_candidate_url("https://cdn.example.com/stream"));
        assert!(is_playlist_type("application/vnd.apple.mpegurl"));
        assert!(is_playlist_type("Application/X-MpegURL; charset=UTF-8"));
        assert!(is_playlist_type("audio/x-mpegurl"));
        assert!(!is_playlist_type("text/plain"));
        assert!(!is_playlist_type("video/mp2t"));
    }

    #[test]
    fn names_lists_and_streams() {
        assert_eq!(list_name("https://cdn.example.com/music/Best%20of%202026.m3u").as_deref(), Some("Best of 2026"));
        let meta = stream_meta("https://cdn.example.com/big-buck-bunny/hls/master.m3u8");
        assert_eq!((meta.title.as_deref(), meta.id), (Some("big-buck-bunny"), None));
    }
}
