//! Pixeldrain files (`/u/<id>`) and lists (`/l/<id>`), through the public API.

use async_trait::async_trait;
use serde_json::Value;
use url::Url;

use super::{FolderEntry, FolderListing, Hoster, HosterError, HosterLink, host_is, segment_after, status_error};
use crate::checksum::{FileHash, HashAlgo};

pub struct Pixeldrain;

const API: &str = "https://pixeldrain.com/api";

#[async_trait]
impl Hoster for Pixeldrain {
    fn name(&self) -> &'static str {
        "pixeldrain"
    }

    fn label(&self) -> &'static str {
        "Pixeldrain"
    }

    fn matches(&self, url: &Url) -> bool {
        host_is(url, &["pixeldrain.com", "pixeldra.in"])
    }

    async fn expand(&self, http: &reqwest::Client, url: &Url) -> Result<Option<FolderListing>, HosterError> {
        let Some(id) = segment_after(url, "l") else { return Ok(None) };
        let response = http.get(format!("{API}/list/{id}")).send().await?;
        if !response.status().is_success() {
            return Err(status_error(response.status()));
        }
        let list: Value = response.json().await?;
        let entries = list["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|file| {
                Some(FolderEntry {
                    url: format!("https://pixeldrain.com/u/{}", file["id"].as_str()?),
                    name: file["name"].as_str().map(str::to_owned),
                    size: file["size"].as_u64(),
                    rel_dir: String::new(),
                })
            })
            .collect();
        Ok(Some(FolderListing { name: list["title"].as_str().map(str::to_owned), entries }))
    }

    async fn resolve(&self, http: &reqwest::Client, url: &Url) -> Result<HosterLink, HosterError> {
        let id = segment_after(url, "u")
            .or_else(|| segment_after(url, "file"))
            .ok_or_else(|| HosterError::Failed("unrecognised Pixeldrain link".into()))?;
        let response = http.get(format!("{API}/file/{id}/info")).send().await?;
        if !response.status().is_success() {
            return Err(status_error(response.status()));
        }
        let info: Value = response.json().await?;
        Ok(HosterLink {
            filename: info["name"].as_str().map(str::to_owned),
            size: info["size"].as_u64(),
            hash: file_hash(&info),
            ..HosterLink::direct(format!("{API}/file/{id}?download"))
        })
    }
}

/// `hash_sha256` of the file info.
fn file_hash(info: &Value) -> Option<FileHash> {
    info["hash_sha256"].as_str().and_then(|hash| FileHash::new(HashAlgo::Sha256, hash))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_published_sha256() {
        let info = serde_json::json!({
            "success": true,
            "id": "1234abcd",
            "name": "screenshot.png",
            "size": 5694837,
            "hash_sha256": "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        });
        let hash = file_hash(&info).unwrap();
        assert_eq!(hash.algo, HashAlgo::Sha256);
        assert_eq!(hash.hex, "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
        assert_eq!(file_hash(&serde_json::json!({ "name": "x" })), None);
    }
}
