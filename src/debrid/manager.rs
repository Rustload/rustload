//! Account selection, supported-host caching and fallback between providers.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use tokio::sync::{Mutex, RwLock};

use super::{AccountStatus, DebridError, DebridService, HostMatcher, LinkInfo, RemoteFile, TransferStatus, Unlocked};
use crate::{
    db::{self, Db},
    models::AccountRow,
};

/// How long a provider's supported-host list is trusted.
const HOSTS_TTL: Duration = Duration::from_secs(6 * 3600);
/// After a failure to fetch the host list, wait this long before asking again.
const HOSTS_RETRY: Duration = Duration::from_secs(10 * 60);
/// An account that hit its quota is skipped for this long (other accounts are tried first).
const QUOTA_COOLDOWN: Duration = Duration::from_secs(30 * 60);
/// Same after a temporary failure.
const TEMPORARY_COOLDOWN: Duration = Duration::from_secs(2 * 60);

/// Supported-host list of an account and when it was fetched (`None` if fetching failed).
type CachedHosts = (Instant, Option<Arc<HostMatcher>>);

/// An enabled account together with its service client. Provider calls go through its methods,
/// which store the credentials the provider renewed meanwhile (refreshed OAuth token, new
/// session), so that they survive reloads and restarts.
#[derive(Clone)]
pub struct AccountHandle {
    pub id: i64,
    pub label: String,
    pub provider: &'static str,
    pub magnets: bool,
    service: Arc<dyn DebridService>,
    db: Db,
}

impl AccountHandle {
    pub async fn account_status(&self) -> Result<AccountStatus, DebridError> {
        self.persisted(self.service.account_status().await).await
    }

    pub async fn supported_hosts(&self) -> Result<HostMatcher, DebridError> {
        self.persisted(self.service.supported_hosts().await).await
    }

    pub async fn unlock(&self, link: &str, password: Option<&str>) -> Result<Unlocked, DebridError> {
        self.persisted(self.service.unlock(link, password).await).await
    }

    pub async fn link_info(&self, link: &str) -> Result<Option<LinkInfo>, DebridError> {
        self.persisted(self.service.link_info(link).await).await
    }

    pub async fn add_magnet(&self, magnet: &str) -> Result<String, DebridError> {
        self.persisted(self.service.add_magnet(magnet).await).await
    }

    pub async fn add_torrent(&self, file_name: &str, data: Vec<u8>) -> Result<String, DebridError> {
        self.persisted(self.service.add_torrent(file_name, data).await).await
    }

    pub async fn transfer_status(&self, id: &str) -> Result<TransferStatus, DebridError> {
        self.persisted(self.service.transfer_status(id).await).await
    }

    pub async fn transfer_files(&self, id: &str) -> Result<Vec<RemoteFile>, DebridError> {
        self.persisted(self.service.transfer_files(id).await).await
    }

    pub async fn delete_transfer(&self, id: &str) -> Result<(), DebridError> {
        self.persisted(self.service.delete_transfer(id).await).await
    }

    /// Returns `result` once the credentials renewed during the call, if any, are stored.
    async fn persisted<T>(&self, result: T) -> T {
        if let Some(credentials) = self.service.take_updated_credentials()
            && let Err(error) = db::accounts::update_credentials(&self.db, self.id, &credentials).await
        {
            tracing::warn!("cannot store refreshed credentials of `{}`: {error}", self.label);
        }
        result
    }
}

pub struct DebridManager {
    db: Db,
    http: reqwest::Client,
    accounts: RwLock<Vec<AccountHandle>>,
    hosts: Mutex<HashMap<i64, CachedHosts>>,
    /// Accounts temporarily skipped, until the given instant.
    cooldowns: std::sync::Mutex<HashMap<i64, Instant>>,
}

impl DebridManager {
    pub async fn new(db: Db, http: reqwest::Client) -> Result<Self> {
        let manager = Self {
            db,
            http,
            accounts: RwLock::new(Vec::new()),
            hosts: Mutex::new(HashMap::new()),
            cooldowns: Default::default(),
        };
        manager.reload().await?;
        Ok(manager)
    }

    /// Rebuilds clients from the database (after any account change).
    pub async fn reload(&self) -> Result<()> {
        let rows = db::accounts::list(&self.db).await?;
        let mut handles = Vec::new();
        for row in rows.into_iter().filter(|r| r.enabled) {
            match self.handle(&row) {
                Ok(handle) => handles.push(handle),
                Err(error) => tracing::warn!("account `{}` ignored: {error}", row.label),
            }
        }
        *self.accounts.write().await = handles;
        self.hosts.lock().await.clear();
        Ok(())
    }

    fn handle(&self, row: &AccountRow) -> Result<AccountHandle, DebridError> {
        let service: Arc<dyn DebridService> = super::build(&row.provider, &row.credentials, self.http.clone())?.into();
        let info = service.info();
        Ok(AccountHandle {
            id: row.id,
            label: row.label.clone(),
            provider: info.id,
            magnets: info.magnets,
            service,
            db: self.db.clone(),
        })
    }

    pub async fn enabled(&self) -> Vec<AccountHandle> {
        self.accounts.read().await.clone()
    }

    pub async fn get(&self, id: i64) -> Option<AccountHandle> {
        self.accounts.read().await.iter().find(|a| a.id == id).cloned()
    }

    /// First enabled account able to process magnets and torrents.
    pub async fn magnet_account(&self) -> Option<AccountHandle> {
        self.accounts.read().await.iter().find(|a| a.magnets).cloned()
    }

    /// Accounts whose provider supports the host of `url`, in priority order. Accounts in
    /// cooldown come last, so they are only used when nothing else can serve the link.
    pub async fn candidates(&self, url: &str) -> Vec<AccountHandle> {
        let mut result = Vec::new();
        for account in self.enabled().await {
            if let Some(matcher) = self.matcher(&account).await
                && matcher.matches(url)
            {
                result.push(account);
            }
        }
        let cooldowns = self.cooldowns.lock().expect("poisoned");
        let now = Instant::now();
        result.sort_by_key(|a| cooldowns.get(&a.id).is_some_and(|until| *until > now));
        result
    }

    fn cool_down(&self, account: &AccountHandle, error: &DebridError) {
        let duration = match error {
            DebridError::Quota(_) => QUOTA_COOLDOWN,
            DebridError::Temporary(_) => TEMPORARY_COOLDOWN,
            _ => return,
        };
        self.cooldowns.lock().expect("poisoned").insert(account.id, Instant::now() + duration);
    }

    async fn matcher(&self, account: &AccountHandle) -> Option<Arc<HostMatcher>> {
        if let Some((fetched, matcher)) = self.hosts.lock().await.get(&account.id) {
            let ttl = if matcher.is_some() { HOSTS_TTL } else { HOSTS_RETRY };
            if fetched.elapsed() < ttl {
                return matcher.clone();
            }
        }
        let matcher = match account.supported_hosts().await {
            Ok(matcher) => Some(Arc::new(matcher)),
            Err(error) => {
                tracing::warn!("cannot fetch supported hosts of `{}`: {error}", account.label);
                None
            }
        };
        self.hosts.lock().await.insert(account.id, (Instant::now(), matcher.clone()));
        matcher
    }

    /// Unlocks `url` with the first account that can. `Ok(None)` means no account handles this
    /// host and the link should be downloaded directly.
    pub async fn unlock(
        &self,
        url: &str,
        password: Option<&str>,
    ) -> Result<Option<(Unlocked, AccountHandle)>, DebridError> {
        let candidates = self.candidates(url).await;
        if candidates.is_empty() {
            return Ok(None);
        }
        let mut last_error = None;
        for account in candidates {
            match self.unlock_with(&account, url, password).await {
                Ok(unlocked) => return Ok(Some((unlocked, account))),
                Err(DebridError::LinkDown(message)) => return Err(DebridError::LinkDown(message)),
                Err(error) => {
                    tracing::info!("`{}` could not unlock {url}: {error}", account.label);
                    last_error = Some(error);
                }
            }
        }
        match last_error {
            Some(DebridError::Unsupported) | None => Ok(None),
            Some(error) => Err(error),
        }
    }

    /// Unlocks through one specific account, recording authentication problems on it.
    pub async fn unlock_with(
        &self,
        account: &AccountHandle,
        url: &str,
        password: Option<&str>,
    ) -> Result<Unlocked, DebridError> {
        let result = account.unlock(url, password).await;
        match &result {
            Err(DebridError::Auth(message)) => {
                let _ = db::accounts::record_check(&self.db, account.id, Err(message)).await;
            }
            Err(error) => self.cool_down(account, error),
            Ok(_) => {}
        }
        result
    }

    /// Refreshes the stored premium status of an account (enabled or not).
    pub async fn check(&self, row: &AccountRow) -> Result<(), DebridError> {
        let handle = self.handle(row)?;
        let result = handle.account_status().await;
        let recorded = match &result {
            Ok(status) => db::accounts::record_check(&self.db, row.id, Ok(status)).await,
            Err(error) => db::accounts::record_check(&self.db, row.id, Err(&error.to_string())).await,
        };
        if let Err(error) = recorded {
            tracing::warn!("cannot store account status: {error}");
        }
        result.map(|_| ())
    }

    /// Checks every account; called periodically.
    pub async fn check_all(&self) {
        let Ok(rows) = db::accounts::list(&self.db).await else { return };
        for row in rows.iter().filter(|r| r.enabled) {
            if let Err(error) = self.check(row).await {
                tracing::warn!("account `{}`: {error}", row.label);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;

    use super::*;
    use crate::debrid::ProviderInfo;

    /// A service whose access token is renewed by any call.
    #[derive(Default)]
    struct Refreshing {
        updated: std::sync::Mutex<Option<String>>,
    }

    #[async_trait]
    impl DebridService for Refreshing {
        fn info(&self) -> ProviderInfo {
            crate::debrid::provider("realdebrid").unwrap()
        }

        async fn account_status(&self) -> Result<AccountStatus, DebridError> {
            Err(DebridError::Unsupported)
        }

        async fn supported_hosts(&self) -> Result<HostMatcher, DebridError> {
            Ok(HostMatcher::default())
        }

        async fn unlock(&self, _link: &str, _password: Option<&str>) -> Result<Unlocked, DebridError> {
            Err(DebridError::Unsupported)
        }

        async fn transfer_status(&self, _id: &str) -> Result<TransferStatus, DebridError> {
            *self.updated.lock().unwrap() = Some(r#"{"access_token":"renewed"}"#.into());
            Err(DebridError::Temporary("busy".into()))
        }

        fn take_updated_credentials(&self) -> Option<String> {
            self.updated.lock().unwrap().take()
        }
    }

    #[tokio::test]
    async fn stores_credentials_renewed_by_any_call() {
        let dir = tempfile::tempdir().unwrap();
        let db = db::open(&dir.path().join("rustload.db")).await.unwrap();
        let id = db::accounts::insert(&db, "realdebrid", "RD", r#"{"access_token":"old"}"#, true).await.unwrap();
        let account = AccountHandle {
            id,
            label: "RD".into(),
            provider: "realdebrid",
            magnets: true,
            service: Arc::new(Refreshing::default()),
            db: db.clone(),
        };

        // Even a failed call may have renewed the token first.
        assert!(account.transfer_status("1").await.is_err());
        let stored = db::accounts::get(&db, id).await.unwrap().unwrap().credentials;
        assert_eq!(stored, r#"{"access_token":"renewed"}"#);
        db.close().await;
    }
}
