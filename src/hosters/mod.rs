//! Built-in hoster plugins.
//!
//! A plugin turns a share link into a direct download (`resolve`) and can expand folder links
//! into their files (`expand`). Plugins are used when no debrid account handles the host; they
//! never touch the downloaded bytes themselves, except through an optional [`Decryptor`] applied
//! by the transfer engine (Mega files are encrypted).

mod dropbox;
mod gdrive;
mod gofile;
mod mediafire;
mod mega;
mod pixeldrain;

use std::sync::Arc;

use async_trait::async_trait;
use reqwest::header::HeaderMap;
use url::Url;

use crate::{checksum::FileHash, util::host_matches};

/// Decrypts file content in place; `offset` is the position of `data` in the file.
pub trait Decryptor: Send + Sync {
    fn apply(&self, offset: u64, data: &mut [u8]);
}

/// A direct download produced by a plugin.
pub struct HosterLink {
    pub url: String,
    pub filename: Option<String>,
    pub size: Option<u64>,
    pub headers: HeaderMap,
    pub decryptor: Option<Arc<dyn Decryptor>>,
    /// Digest of the file, when the hoster publishes one (checked after the download).
    pub hash: Option<FileHash>,
}

impl HosterLink {
    fn direct(url: impl Into<String>) -> Self {
        Self { url: url.into(), filename: None, size: None, headers: HeaderMap::new(), decryptor: None, hash: None }
    }
}

/// Content of a folder link.
#[derive(Debug, Clone, PartialEq)]
pub struct FolderListing {
    /// Name of the shared folder, used as package name when known.
    pub name: Option<String>,
    pub entries: Vec<FolderEntry>,
}

/// A file found inside a folder link.
#[derive(Debug, Clone, PartialEq)]
pub struct FolderEntry {
    pub url: String,
    pub name: Option<String>,
    pub size: Option<u64>,
    /// Sub-folder inside the shared folder, `/`-separated, empty at the top level.
    pub rel_dir: String,
}

#[derive(Debug, thiserror::Error)]
pub enum HosterError {
    #[error("file not available: {0}")]
    Offline(String),
    #[error("{0}")]
    Temporary(String),
    #[error("{0}")]
    Failed(String),
}

impl From<reqwest::Error> for HosterError {
    fn from(error: reqwest::Error) -> Self {
        Self::Temporary(format!("network error: {}", error.without_url()))
    }
}

/// Maps a non-success HTTP status of a hoster page or API.
fn status_error(status: reqwest::StatusCode) -> HosterError {
    match status.as_u16() {
        404 | 410 => HosterError::Offline(format!("HTTP {status}")),
        429 | 500..=599 => HosterError::Temporary(format!("HTTP {status}")),
        _ => HosterError::Failed(format!("HTTP {status}")),
    }
}

#[async_trait]
pub trait Hoster: Send + Sync {
    fn name(&self) -> &'static str;
    /// Name shown to users, the plugin name by default.
    fn label(&self) -> &'static str {
        self.name()
    }
    fn matches(&self, url: &Url) -> bool;
    /// `Some(files)` when `url` is a folder or collection, `None` for a single file.
    async fn expand(&self, _http: &reqwest::Client, _url: &Url) -> Result<Option<FolderListing>, HosterError> {
        Ok(None)
    }
    async fn resolve(&self, http: &reqwest::Client, url: &Url) -> Result<HosterLink, HosterError>;
}

pub struct Hosters {
    plugins: Vec<Box<dyn Hoster>>,
}

impl Default for Hosters {
    fn default() -> Self {
        Self {
            plugins: vec![
                Box::new(mega::Mega),
                Box::new(gdrive::GoogleDrive),
                Box::new(dropbox::Dropbox),
                Box::new(mediafire::MediaFire),
                Box::new(pixeldrain::Pixeldrain),
                Box::new(gofile::Gofile::default()),
            ],
        }
    }
}

impl Hosters {
    pub fn find(&self, url: &str) -> Option<&dyn Hoster> {
        let url = Url::parse(url).ok()?;
        self.plugins.iter().find(|p| p.matches(&url)).map(Box::as_ref)
    }
}

/// `true` if `url`'s host is one of `domains` or one of their sub-domains.
fn host_is(url: &Url, domains: &[&str]) -> bool {
    let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
    domains.iter().any(|domain| host_matches(&host, domain))
}

/// First path segment after `prefix`, e.g. `/file/d/<id>/view` with prefix `d` → `<id>`.
fn segment_after<'a>(url: &'a Url, prefix: &str) -> Option<&'a str> {
    let mut segments = url.path_segments()?;
    segments.by_ref().find(|s| *s == prefix)?;
    segments.next().filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_plugins_by_url() {
        let hosters = Hosters::default();
        let name = |url: &str| hosters.find(url).map(|h| h.name());
        assert_eq!(name("https://mega.nz/file/abc#key"), Some("mega"));
        assert_eq!(name("https://drive.google.com/file/d/xyz/view"), Some("gdrive"));
        assert_eq!(name("https://www.dropbox.com/s/abc/file.zip?dl=0"), Some("dropbox"));
        assert_eq!(name("https://www.mediafire.com/file/abc/file.zip/file"), Some("mediafire"));
        assert_eq!(name("https://pixeldrain.com/u/abc"), Some("pixeldrain"));
        assert_eq!(name("https://gofile.io/d/abc"), Some("gofile"));
        assert_eq!(name("https://example.com/file.zip"), None);
    }

    #[test]
    fn reads_path_segments() {
        let url = Url::parse("https://drive.google.com/file/d/ID123/view?usp=sharing").unwrap();
        assert_eq!(segment_after(&url, "d"), Some("ID123"));
        assert!(host_is(&url, &["google.com"]));
    }
}
