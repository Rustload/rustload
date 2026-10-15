//! What yt-dlp reports about a link (`-J` output and `--print` lines), and the shapes Rustload
//! keeps from it.

use serde::{Deserialize, Serialize};

/// Entries kept from a playlist (and listed by the probe endpoint).
pub const MAX_ENTRIES: usize = 500;

/// The subset of yt-dlp's info dictionary Rustload reads. Every field is optional: extractors
/// fill what their site provides.
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct RawInfo {
    #[serde(rename = "_type")]
    pub kind: Option<String>,
    pub id: Option<String>,
    pub title: Option<String>,
    pub uploader: Option<String>,
    pub channel: Option<String>,
    pub duration: Option<f64>,
    pub thumbnail: Option<String>,
    pub thumbnails: Vec<Thumbnail>,
    pub webpage_url: Option<String>,
    pub original_url: Option<String>,
    pub url: Option<String>,
    pub extractor: Option<String>,
    pub extractor_key: Option<String>,
    pub ie_key: Option<String>,
    pub is_live: Option<bool>,
    pub live_status: Option<String>,
    pub ext: Option<String>,
    pub format_id: Option<String>,
    pub filesize: Option<f64>,
    pub filesize_approx: Option<f64>,
    pub filename: Option<String>,
    pub playlist_count: Option<u64>,
    pub entries: Option<Vec<RawInfo>>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Thumbnail {
    pub url: Option<String>,
}

impl RawInfo {
    pub fn is_playlist(&self) -> bool {
        self.kind.as_deref() == Some("playlist") || self.entries.is_some()
    }

    pub fn is_live(&self) -> bool {
        self.is_live == Some(true) || self.live_status.as_deref() == Some("is_live")
    }

    /// Best size estimate of the selected format(s), in bytes.
    pub fn size_estimate(&self) -> Option<u64> {
        self.filesize.or(self.filesize_approx).filter(|s| *s > 0.0).map(|s| s as u64)
    }

    fn thumbnail_url(&self) -> Option<String> {
        self.thumbnail.clone().or_else(|| self.thumbnails.iter().rev().find_map(|t| t.url.clone()))
    }

    /// Link of a playlist entry, when yt-dlp gives an absolute one.
    fn entry_url(&self) -> Option<String> {
        [&self.url, &self.webpage_url, &self.original_url]
            .into_iter()
            .flatten()
            .find(|u| u.starts_with("https://") || u.starts_with("http://"))
            .cloned()
    }

    /// An entry that is itself a list (a channel tab, a season...), to expand in turn.
    fn is_nested_list(&self, parent_extractor: Option<&str>) -> bool {
        let ie_key = self.ie_key.as_deref().unwrap_or_default();
        self.kind.as_deref() == Some("playlist")
            || ie_key.ends_with("Tab")
            || ie_key.ends_with("Playlist")
            || (!ie_key.is_empty() && Some(ie_key) == parent_extractor)
    }

    pub fn meta(&self) -> MediaMeta {
        MediaMeta {
            id: self.id.clone(),
            title: self.title.clone(),
            duration_secs: seconds(self.duration),
            thumbnail: self.thumbnail_url(),
            uploader: self.uploader.clone().or_else(|| self.channel.clone()),
            extractor: self.extractor.clone().or_else(|| self.ie_key.as_ref().map(|k| k.to_ascii_lowercase())),
        }
    }

    pub fn into_media_info(mut self) -> MediaInfo {
        let meta = self.meta();
        let is_live = self.is_live();
        let filesize_approx = self.size_estimate();
        let is_playlist = self.is_playlist();
        // Lists inside the list (the tabs of a channel: videos, shorts, lives) come with their
        // entries: list those directly.
        let mut raw_entries = Vec::new();
        let mut flattened = false;
        for mut entry in self.entries.take().unwrap_or_default() {
            match entry.entries.take() {
                Some(children) => {
                    flattened = true;
                    raw_entries.extend(children.into_iter().filter(|child| child.entries.is_none()));
                }
                None => raw_entries.push(entry),
            }
        }
        let playlist = is_playlist.then(|| PlaylistInfo {
            title: self.title.clone(),
            count: match self.playlist_count {
                Some(count) if !flattened => count,
                _ => raw_entries.len() as u64,
            },
        });
        let parent_extractor = self.extractor_key.clone();
        let entries = raw_entries
            .into_iter()
            .filter_map(|entry| {
                let url = entry.entry_url()?;
                Some(MediaEntry {
                    url,
                    title: entry.title.clone(),
                    duration_secs: seconds(entry.duration),
                    nested: entry.is_nested_list(parent_extractor.as_deref()),
                    meta: entry.meta(),
                })
            })
            .take(MAX_ENTRIES)
            .collect();
        MediaInfo {
            title: meta.title.clone(),
            uploader: meta.uploader.clone(),
            duration_secs: meta.duration_secs,
            thumbnail: meta.thumbnail.clone(),
            webpage_url: self.webpage_url.or(self.original_url),
            extractor: meta.extractor.clone(),
            is_live,
            playlist,
            entries,
            filesize_approx,
            id: meta.id,
            ext: self.ext,
        }
    }
}

fn seconds(duration: Option<f64>) -> Option<u64> {
    duration.filter(|d| d.is_finite() && *d >= 0.0).map(|d| d.round() as u64)
}

/// Result of `POST /api/media/probe` and of [`super::probe`].
#[derive(Debug, Clone, Serialize)]
pub struct MediaInfo {
    pub title: Option<String>,
    pub uploader: Option<String>,
    pub duration_secs: Option<u64>,
    pub thumbnail: Option<String>,
    pub webpage_url: Option<String>,
    /// yt-dlp extractor, e.g. `youtube`.
    pub extractor: Option<String>,
    pub is_live: bool,
    pub playlist: Option<PlaylistInfo>,
    /// Playlist entries, at most [`MAX_ENTRIES`]; empty for a single video.
    pub entries: Vec<MediaEntry>,
    /// Estimated size of the selected format(s) of a single video.
    pub filesize_approx: Option<u64>,
    #[serde(skip)]
    pub id: Option<String>,
    #[serde(skip)]
    pub ext: Option<String>,
}

impl MediaInfo {
    pub fn is_playlist(&self) -> bool {
        self.playlist.is_some()
    }

    pub fn meta(&self) -> MediaMeta {
        MediaMeta {
            id: self.id.clone(),
            title: self.title.clone(),
            duration_secs: self.duration_secs,
            thumbnail: self.thumbnail.clone(),
            uploader: self.uploader.clone(),
            extractor: self.extractor.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PlaylistInfo {
    pub title: Option<String>,
    /// Total number of entries reported by the site (may exceed the entries listed).
    pub count: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MediaEntry {
    pub url: String,
    pub title: Option<String>,
    pub duration_secs: Option<u64>,
    /// A list inside the list (channel tab): expanded separately.
    #[serde(skip)]
    pub nested: bool,
    #[serde(skip)]
    pub meta: MediaMeta,
}

/// What is known about the video behind a download, stored with it and shown in the queue.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct MediaMeta {
    /// Identifier on the site (YouTube video id...).
    pub id: Option<String>,
    pub title: Option<String>,
    pub duration_secs: Option<u64>,
    pub thumbnail: Option<String>,
    pub uploader: Option<String>,
    pub extractor: Option<String>,
}

/// One format announced by `--print video:` (`requested_formats` of merged downloads).
#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct FormatSize {
    pub format_id: Option<String>,
    pub filesize: Option<f64>,
    pub filesize_approx: Option<f64>,
}

impl FormatSize {
    pub fn size(&self) -> Option<u64> {
        self.filesize.or(self.filesize_approx).filter(|s| *s > 0.0).map(|s| s as u64)
    }
}

/// File name yt-dlp would give a video with the output template Rustload uses
/// (`%(title).180B [%(id)s].%(ext)s`), before sanitising. Used to name links early.
pub fn predicted_name(meta: &MediaMeta, ext: &str) -> Option<String> {
    let title = meta.title.as_deref().map(str::trim).filter(|t| !t.is_empty())?;
    let mut cut = title.len().min(180);
    while !title.is_char_boundary(cut) {
        cut -= 1;
    }
    let title = &title[..cut];
    Some(match &meta.id {
        Some(id) => format!("{title} [{id}].{ext}"),
        None => format!("{title}.{ext}"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_a_single_video() {
        let raw: RawInfo = serde_json::from_str(
            r#"{"_type":"video","id":"jNQXAC9IVRw","title":"Me at the zoo","uploader":"jawed","duration":19,
                "thumbnail":"https://i.ytimg.com/vi/x/hq.jpg","webpage_url":"https://www.youtube.com/watch?v=jNQXAC9IVRw",
                "extractor":"youtube","extractor_key":"Youtube","is_live":false,"live_status":"not_live","ext":"webm",
                "format_id":"395+251","filesize":null,"filesize_approx":475961,"formats":[{"format_id":"18"}]}"#,
        )
        .unwrap();
        assert!(!raw.is_playlist());
        assert!(!raw.is_live());
        assert_eq!(raw.size_estimate(), Some(475_961));
        let info = raw.into_media_info();
        assert_eq!(info.title.as_deref(), Some("Me at the zoo"));
        assert_eq!(info.duration_secs, Some(19));
        assert_eq!(info.extractor.as_deref(), Some("youtube"));
        assert!(info.playlist.is_none());
        assert!(info.entries.is_empty());
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(json["filesize_approx"], 475_961);
        assert!(json.get("id").is_none());
        assert_eq!(info.meta().id.as_deref(), Some("jNQXAC9IVRw"));
    }

    #[test]
    fn parses_a_flat_playlist() {
        let raw: RawInfo = serde_json::from_str(
            r#"{"_type":"playlist","id":"UC4Q","title":"jawed - Videos","uploader":"jawed","extractor":"youtube:tab",
                "extractor_key":"YoutubeTab","webpage_url":"https://www.youtube.com/@jawed/videos","playlist_count":3,
                "entries":[
                  {"_type":"url","ie_key":"Youtube","id":"a1","url":"https://www.youtube.com/watch?v=a1","title":"One","duration":61.6,
                   "thumbnails":[{"url":"https://i.ytimg.com/small.jpg"},{"url":"https://i.ytimg.com/big.jpg"}]},
                  {"_type":"url","ie_key":"Youtube","id":"a2","url":"a2","title":"Relative"},
                  {"_type":"url","ie_key":"YoutubeTab","url":"https://www.youtube.com/@jawed/shorts","title":"Shorts"}
                ]}"#,
        )
        .unwrap();
        assert!(raw.is_playlist());
        let info = raw.into_media_info();
        let playlist = info.playlist.as_ref().unwrap();
        assert_eq!(playlist.title.as_deref(), Some("jawed - Videos"));
        assert_eq!(playlist.count, 3);
        assert_eq!(info.entries.len(), 2);
        assert_eq!(info.entries[0].duration_secs, Some(62));
        assert!(!info.entries[0].nested);
        assert_eq!(info.entries[0].meta.thumbnail.as_deref(), Some("https://i.ytimg.com/big.jpg"));
        assert!(info.entries[1].nested);
        let json = serde_json::to_value(&info).unwrap();
        assert_eq!(
            json["entries"][0],
            serde_json::json!({"url": "https://www.youtube.com/watch?v=a1", "title": "One", "duration_secs": 62})
        );
    }

    #[test]
    fn lists_channel_tabs_inline() {
        let raw: RawInfo = serde_json::from_str(
            r#"{"_type":"playlist","title":"Chan","extractor_key":"YoutubeTab","playlist_count":2,"entries":[
                  {"_type":"playlist","title":"Chan - Videos","webpage_url":"https://www.youtube.com/@c/videos","entries":[
                    {"_type":"url","ie_key":"Youtube","url":"https://www.youtube.com/watch?v=v1","title":"V1"},
                    {"_type":"url","ie_key":"Youtube","url":"https://www.youtube.com/watch?v=v2","title":"V2"}]},
                  {"_type":"playlist","title":"Chan - Shorts","webpage_url":"https://www.youtube.com/@c/shorts","entries":[
                    {"_type":"url","ie_key":"Youtube","url":"https://www.youtube.com/shorts/s1","title":"S1"}]}
                ]}"#,
        )
        .unwrap();
        let info = raw.into_media_info();
        let urls: Vec<&str> = info.entries.iter().map(|e| e.url.as_str()).collect();
        assert_eq!(
            urls,
            [
                "https://www.youtube.com/watch?v=v1",
                "https://www.youtube.com/watch?v=v2",
                "https://www.youtube.com/shorts/s1"
            ]
        );
        assert!(info.entries.iter().all(|e| !e.nested));
        assert_eq!(info.playlist.unwrap().count, 3);
    }

    #[test]
    fn predicts_names_like_yt_dlp() {
        let meta = MediaMeta { id: Some("abc".into()), title: Some("Clip".into()), ..MediaMeta::default() };
        assert_eq!(predicted_name(&meta, "mp4").as_deref(), Some("Clip [abc].mp4"));
        let long = MediaMeta { title: Some("é".repeat(100)), ..MediaMeta::default() };
        let name = predicted_name(&long, "m4a").unwrap();
        assert!(name.len() <= 184 && name.ends_with(".m4a"));
        assert_eq!(predicted_name(&MediaMeta::default(), "mp4"), None);
    }
}
