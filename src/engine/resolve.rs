//! Turns a user link into a direct download URL: pinned debrid account, then debrid accounts in
//! priority order, then a built-in hoster plugin, then the link itself.

use std::sync::Arc;

use reqwest::header::HeaderMap;

use super::{Engine, task::TaskError};
use crate::{
    checksum::FileHash,
    debrid::{DebridError, Unlocked},
    hosters::{Decryptor, HosterError},
    models::DownloadRow,
};

pub const DIRECT: &str = "direct";

pub struct Resolved {
    pub url: String,
    pub filename: Option<String>,
    pub size: Option<u64>,
    /// `direct`, the debrid provider id or the hoster plugin name.
    pub via: String,
    pub headers: HeaderMap,
    pub decryptor: Option<Arc<dyn Decryptor>>,
    /// Digest published by the provider or plugin.
    pub hash: Option<FileHash>,
}

impl Resolved {
    fn new(url: String, filename: Option<String>, size: Option<u64>, via: &str) -> Self {
        Self { url, filename, size, via: via.to_owned(), headers: HeaderMap::new(), decryptor: None, hash: None }
    }

    fn unlocked(unlocked: Unlocked, via: &str) -> Self {
        Self { hash: unlocked.hash, ..Self::new(unlocked.url, unlocked.filename, unlocked.size, via) }
    }

    pub fn is_direct(&self) -> bool {
        self.via == DIRECT
    }
}

pub async fn resolve(engine: &Engine, download: &DownloadRow) -> Result<Resolved, TaskError> {
    // Files of a debrid transfer must be unlocked by the account that holds the transfer.
    if let Some(account_id) = download.account_id {
        let account =
            engine.debrid.get(account_id).await.ok_or_else(|| {
                TaskError::Fatal("the debrid account holding this file is disabled or deleted".into())
            })?;
        let unlocked = engine.debrid.unlock_with(&account, &download.url, None).await.map_err(into_task_error)?;
        return Ok(Resolved::unlocked(unlocked, account.provider));
    }

    if engine.settings.get().debrid.enabled {
        match engine.debrid.unlock(&download.url, None).await {
            Ok(Some((unlocked, account))) => return Ok(Resolved::unlocked(unlocked, account.provider)),
            Ok(None) => {}
            Err(error) => return Err(into_task_error(error)),
        }
    }

    if let Some(hoster) = engine.hosters.find(&download.url) {
        let url = url::Url::parse(&download.url).map_err(|e| TaskError::Fatal(e.to_string()))?;
        let link = hoster.resolve(&engine.download_client(), &url).await.map_err(hoster_error)?;
        return Ok(Resolved {
            url: link.url,
            filename: link.filename,
            size: link.size,
            via: hoster.name().into(),
            headers: link.headers,
            decryptor: link.decryptor,
            hash: link.hash,
        });
    }

    Ok(Resolved::new(download.url.clone(), None, None, DIRECT))
}

fn hoster_error(error: HosterError) -> TaskError {
    match error {
        HosterError::Temporary(message) => TaskError::Transient(message),
        other => TaskError::Fatal(other.to_string()),
    }
}

fn into_task_error(error: DebridError) -> TaskError {
    match error {
        DebridError::LinkDown(message) => TaskError::Fatal(format!("link unavailable: {message}")),
        DebridError::Unsupported => TaskError::Fatal("no debrid account supports this host".into()),
        DebridError::Auth(message) => TaskError::Transient(format!("debrid account error: {message}")),
        other => TaskError::Transient(other.to_string()),
    }
}
