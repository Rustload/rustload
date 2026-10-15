//! MediaFire files (download button of the file page) and folders (public API).

use std::sync::OnceLock;

use async_trait::async_trait;
use regex::Regex;
use serde_json::Value;
use url::Url;

use super::{FolderEntry, FolderListing, Hoster, HosterError, HosterLink, host_is, segment_after, status_error};
use crate::util::join_relative;

pub struct MediaFire;

const API: &str = "https://www.mediafire.com/api/1.5";
/// Pages of 100 items fetched per folder at most.
const MAX_CHUNKS: usize = 50;

#[async_trait]
impl Hoster for MediaFire {
    fn name(&self) -> &'static str {
        "mediafire"
    }

    fn label(&self) -> &'static str {
        "MediaFire"
    }

    fn matches(&self, url: &Url) -> bool {
        host_is(url, &["mediafire.com"])
    }

    async fn expand(&self, http: &reqwest::Client, url: &Url) -> Result<Option<FolderListing>, HosterError> {
        let Some(key) = segment_after(url, "folder").map(str::to_owned) else { return Ok(None) };
        let mut entries = Vec::new();
        let mut pending = vec![(key, String::new())];
        while let Some((key, prefix)) = pending.pop() {
            for file in folder_items(http, &key, "files").await? {
                entries.push(FolderEntry {
                    url: format!("https://www.mediafire.com/file/{}", file["quickkey"].as_str().unwrap_or_default()),
                    name: file["filename"].as_str().map(str::to_owned),
                    size: file["size"].as_str().and_then(|s| s.parse().ok()),
                    rel_dir: prefix.clone(),
                });
            }
            for folder in folder_items(http, &key, "folders").await? {
                let name = folder["name"].as_str().unwrap_or("folder");
                if let Some(child) = folder["folderkey"].as_str() {
                    let path = join_relative(&prefix, name);
                    pending.push((child.to_owned(), path));
                }
            }
        }
        Ok(Some(FolderListing { name: None, entries }))
    }

    async fn resolve(&self, http: &reqwest::Client, url: &Url) -> Result<HosterLink, HosterError> {
        let page = http.get(url.as_str()).send().await?;
        if !page.status().is_success() {
            return Err(status_error(page.status()));
        }
        let html = page.text().await?;
        if html.contains("error.php?errno=") || html.contains("File Removed") {
            return Err(HosterError::Offline("the file was removed".into()));
        }
        download_button(&html)
            .map(HosterLink::direct)
            .ok_or_else(|| HosterError::Failed("download link not found on the MediaFire page".into()))
    }
}

/// Every item of one kind (`files` or `folders`) in a folder, following the API pagination.
async fn folder_items(http: &reqwest::Client, key: &str, kind: &str) -> Result<Vec<Value>, HosterError> {
    let mut items = Vec::new();
    for chunk in 1..=MAX_CHUNKS {
        let response = http
            .get(format!("{API}/folder/get_content.php"))
            .query(&[
                ("folder_key", key),
                ("content_type", kind),
                ("response_format", "json"),
                ("chunk", &chunk.to_string()),
            ])
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(status_error(response.status()));
        }
        let body: Value = response.json().await?;
        let content = &body["response"]["folder_content"];
        if body["response"]["result"] != "Success" {
            let message = body["response"]["message"].as_str().unwrap_or("folder not found");
            return Err(HosterError::Offline(message.to_owned()));
        }
        items.extend(content[kind].as_array().cloned().unwrap_or_default());
        if content["more_chunks"] != "yes" {
            break;
        }
    }
    Ok(items)
}

fn download_button(html: &str) -> Option<String> {
    static BUTTON: OnceLock<Regex> = OnceLock::new();
    let button = BUTTON.get_or_init(|| {
        Regex::new(r#"(?:aria-label="Download file"|id="downloadButton")[^>]*?href="(https?://download[^"]+)""#)
            .expect("valid regex")
    });
    static FALLBACK: OnceLock<Regex> = OnceLock::new();
    let fallback = FALLBACK
        .get_or_init(|| Regex::new(r#"href="(https?://download\d*\.mediafire\.com/[^"]+)""#).expect("valid regex"));
    button.captures(html).or_else(|| fallback.captures(html)).map(|c| c[1].to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_download_button() {
        let html = r#"<a class="input popsok" aria-label="Download file" href="https://download1234.mediafire.com/abc/file.zip" id="downloadButton">"#;
        assert_eq!(download_button(html).as_deref(), Some("https://download1234.mediafire.com/abc/file.zip"));
        assert_eq!(download_button("<html></html>"), None);
    }
}
