//! Google Drive public files and folders, without an API key.
//!
//! Files are fetched from `drive.usercontent.google.com` with `confirm=t`, which skips the "too
//! large to scan for viruses" page. Folders are listed through the embedded folder view.

use std::sync::OnceLock;

use async_trait::async_trait;
use regex::Regex;
use url::Url;

use super::{FolderEntry, FolderListing, Hoster, HosterError, HosterLink, host_is, segment_after, status_error};
use crate::util::{decode_entities, join_relative};

pub struct GoogleDrive;

/// Folders nested deeper than this are not explored.
const MAX_DEPTH: usize = 8;

#[async_trait]
impl Hoster for GoogleDrive {
    fn name(&self) -> &'static str {
        "gdrive"
    }

    fn label(&self) -> &'static str {
        "Google Drive"
    }

    fn matches(&self, url: &Url) -> bool {
        host_is(url, &["drive.google.com", "docs.google.com", "drive.usercontent.google.com"])
    }

    async fn expand(&self, http: &reqwest::Client, url: &Url) -> Result<Option<FolderListing>, HosterError> {
        let Some(folder_id) = folder_id(url) else { return Ok(None) };
        let mut entries = Vec::new();
        let mut pending = vec![(folder_id, String::new(), 0)];
        while let Some((id, prefix, depth)) = pending.pop() {
            let page =
                http.get("https://drive.google.com/embeddedfolderview").query(&[("id", id.as_str())]).send().await?;
            if !page.status().is_success() {
                return Err(status_error(page.status()));
            }
            for item in parse_folder(&page.text().await?) {
                let path = join_relative(&prefix, &item.name);
                if item.folder {
                    if depth < MAX_DEPTH {
                        pending.push((item.id, path, depth + 1));
                    }
                } else {
                    entries.push(FolderEntry {
                        url: format!("https://drive.google.com/file/d/{}/view", item.id),
                        name: Some(item.name),
                        size: None,
                        rel_dir: prefix.clone(),
                    });
                }
            }
        }
        if entries.is_empty() {
            return Err(HosterError::Offline("the folder is empty or not shared publicly".into()));
        }
        Ok(Some(FolderListing { name: None, entries }))
    }

    async fn resolve(&self, _http: &reqwest::Client, url: &Url) -> Result<HosterLink, HosterError> {
        let id = file_id(url).ok_or_else(|| HosterError::Failed("unrecognised Google Drive link".into()))?;
        Ok(HosterLink::direct(format!(
            "https://drive.usercontent.google.com/download?id={id}&export=download&confirm=t"
        )))
    }
}

fn file_id(url: &Url) -> Option<String> {
    segment_after(url, "d")
        .map(str::to_owned)
        .or_else(|| url.query_pairs().find(|(k, _)| k == "id").map(|(_, v)| v.into_owned()))
}

fn folder_id(url: &Url) -> Option<String> {
    if url.path().contains("/folders/") {
        return segment_after(url, "folders").map(str::to_owned);
    }
    (url.path().contains("embeddedfolderview"))
        .then(|| url.query_pairs().find(|(k, _)| k == "id").map(|(_, v)| v.into_owned()))
        .flatten()
}

struct Item {
    id: String,
    name: String,
    folder: bool,
}

/// Entries of the embedded folder view: `<div class="flip-entry" id="entry-ID"> ... <a href=...>
/// ... <div class="flip-entry-title">NAME</div>`.
fn parse_folder(html: &str) -> Vec<Item> {
    static ENTRY: OnceLock<Regex> = OnceLock::new();
    let entry = ENTRY.get_or_init(|| {
        Regex::new(r#"(?s)id="entry-([\w-]+)".*?href="([^"]+)".*?class="flip-entry-title">([^<]+)<"#)
            .expect("valid regex")
    });
    entry
        .captures_iter(html)
        .map(|c| Item {
            id: c[1].to_owned(),
            folder: c[2].contains("/folders/"),
            name: decode_entities(c[3].trim()).into_owned(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_ids() {
        let file = Url::parse("https://drive.google.com/file/d/1AbC_d-9/view?usp=sharing").unwrap();
        assert_eq!(file_id(&file).as_deref(), Some("1AbC_d-9"));
        let open = Url::parse("https://drive.google.com/open?id=XYZ").unwrap();
        assert_eq!(file_id(&open).as_deref(), Some("XYZ"));
        let folder = Url::parse("https://drive.google.com/drive/folders/FOLDER1?usp=sharing").unwrap();
        assert_eq!(folder_id(&folder).as_deref(), Some("FOLDER1"));
        assert_eq!(folder_id(&file), None);
    }

    #[test]
    fn parses_embedded_folder_view() {
        let html = r#"<div class="flip-entry" id="entry-F1" tabindex="0"><div class="flip-entry-info">
            <a href="https://drive.google.com/file/d/F1/view?usp=drive_web" target="_blank">
            <div class="flip-entry-title">Movie &amp; Co.mkv</div></a></div></div>
            <div class="flip-entry" id="entry-D1"><a href="https://drive.google.com/drive/folders/D1">
            <div class="flip-entry-title">Extras</div></a></div>"#;
        let items = parse_folder(html);
        assert_eq!(items.len(), 2);
        assert_eq!((items[0].id.as_str(), items[0].name.as_str(), items[0].folder), ("F1", "Movie & Co.mkv", false));
        assert_eq!((items[1].id.as_str(), items[1].folder), ("D1", true));
    }
}
