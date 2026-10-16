//! Debrid (multi-hoster) services and premium accounts of hosters.
//!
//! Every provider implements [`DebridService`]; the rest of the application only deals with the
//! normalised types below and with [`DebridManager`], which picks accounts in priority order and
//! falls back to the next one when a provider cannot serve a link. Hoster accounts (1fichier,
//! Rapidgator...) are providers that only support their own domains and no magnets.

mod alldebrid;
mod api_key;
mod debridlink;
mod keep2share;
mod manager;
mod onefichier;
mod premiumize;
mod rapidgator;
mod realdebrid;
mod session;
mod torbox;

use std::{collections::HashSet, fmt::Display, time::Duration};

use async_trait::async_trait;
use regex::Regex;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::{
    checksum::FileHash,
    util::{mask_secret, parent_domains},
};
pub use manager::{AccountHandle, DebridManager};

/// Static description of a provider, exposed to the UI.
#[derive(Debug, Clone, Serialize)]
pub struct ProviderInfo {
    pub id: &'static str,
    pub name: &'static str,
    pub website: &'static str,
    /// Where users find their API key, or manage their account for `credentials` providers.
    pub api_key_url: &'static str,
    pub device_auth: bool,
    pub magnets: bool,
    pub kind: ProviderKind,
    pub auth: AuthKind,
}

/// Debrid services unlock links of many hosts; hoster accounts only those of their own site.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    Debrid,
    Hoster,
}

/// What users enter to add an account.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthKind {
    ApiKey,
    /// Username (e-mail) and password.
    Credentials,
}

pub fn providers() -> Vec<ProviderInfo> {
    let mut providers = vec![
        alldebrid::INFO,
        realdebrid::INFO,
        premiumize::INFO,
        debridlink::INFO,
        torbox::INFO,
        onefichier::INFO,
        rapidgator::INFO,
    ];
    providers.extend(keep2share::PROJECTS.iter().map(|project| project.info.clone()));
    providers
}

pub fn provider(id: &str) -> Option<ProviderInfo> {
    providers().into_iter().find(|p| p.id == id)
}

/// Builds a service instance from the credentials stored for an account.
pub fn build(provider: &str, credentials: &str, http: reqwest::Client) -> Result<Box<dyn DebridService>, DebridError> {
    Ok(match provider {
        alldebrid::ID => Box::new(alldebrid::AllDebrid::from_credentials(credentials, http)?),
        realdebrid::ID => Box::new(realdebrid::RealDebrid::from_credentials(credentials, http)?),
        premiumize::ID => Box::new(premiumize::Premiumize::from_credentials(credentials, http)?),
        debridlink::ID => Box::new(debridlink::DebridLink::from_credentials(credentials, http)?),
        torbox::ID => Box::new(torbox::TorBox::from_credentials(credentials, http)?),
        onefichier::ID => Box::new(onefichier::OneFichier::from_credentials(credentials, http)?),
        rapidgator::ID => Box::new(rapidgator::Rapidgator::from_credentials(credentials, http)?),
        other => match keep2share::project(other) {
            Some(project) => Box::new(keep2share::Keep2Share::from_credentials(project, credentials, http)?),
            None => return Err(unknown(other)),
        },
    })
}

/// Credentials JSON for a plain API key.
pub fn api_key_credentials(provider: &str, api_key: &str) -> Result<String, DebridError> {
    match self::provider(provider) {
        Some(info) if info.auth == AuthKind::ApiKey => Ok(api_key::encode(api_key)),
        Some(_) => Err(DebridError::Other("this provider uses a username and password, not an API key".into())),
        None => Err(unknown(provider)),
    }
}

/// Credentials JSON for a username/password account.
pub fn login_credentials(provider: &str, username: &str, password: &str) -> Result<String, DebridError> {
    match self::provider(provider) {
        Some(info) if info.auth == AuthKind::Credentials => Ok(session::credentials(username, password)),
        Some(_) => Err(DebridError::Other("this provider uses an API key, not a username and password".into())),
        None => Err(unknown(provider)),
    }
}

/// Replaces the username and/or password of stored username/password credentials.
pub fn update_login(credentials: &str, username: Option<&str>, password: Option<&str>) -> Result<String, DebridError> {
    session::update(credentials, username, password)
}

/// Login name stored in username/password credentials.
pub fn login_username(credentials: &str) -> Option<String> {
    session::username(credentials)
}

/// The main secret of stored credentials (API key or OAuth access token), masked. Passwords are
/// never shown, not even partially.
pub fn masked_secret(credentials: &str) -> String {
    let secret = serde_json::from_str::<Value>(credentials)
        .ok()
        .and_then(|c| ["api_key", "access_token"].iter().find_map(|k| c[*k].as_str().map(str::to_owned)))
        .unwrap_or_default();
    mask_secret(&secret)
}

/// Credential fields that change while the account stays the same: the session token of
/// username/password accounts (renewed by logging in) and Real-Debrid's OAuth tokens (refreshed).
const VOLATILE_CREDENTIALS: &[&str] = &["token", "access_token", "refresh_token", "expires_at"];
/// Credential fields that identify an account, by preference: API key, login, OAuth client (a
/// Real-Debrid authorisation has its own client id).
const IDENTITY_CREDENTIALS: &[&str] = &["api_key", "username", "client_id"];

/// Stored credentials as saved in a backup: as JSON (anything else as a JSON string), without the
/// session token of username/password accounts, which is only valid for a while and obtained
/// again by logging in.
pub fn without_session(credentials: &str) -> Value {
    let mut value = serde_json::from_str(credentials).unwrap_or_else(|_| Value::String(credentials.to_owned()));
    if let Value::Object(fields) = &mut value
        && fields.contains_key("password")
    {
        fields.remove("token");
    }
    value
}

/// What tells two accounts of a provider apart, whatever tokens were renewed since `credentials`
/// were saved: the first [`IDENTITY_CREDENTIALS`] field present (logins ignore case and
/// surrounding spaces), else the credentials without their [`VOLATILE_CREDENTIALS`].
pub fn account_identity(credentials: &Value) -> Value {
    let Value::Object(fields) = credentials else { return credentials.clone() };
    for key in IDENTITY_CREDENTIALS {
        if let Some(Value::String(value)) = fields.get(*key) {
            let value = if *key == "username" { value.trim().to_lowercase() } else { value.trim().to_owned() };
            return Value::Object(Map::from_iter([((*key).to_owned(), Value::String(value))]));
        }
    }
    let mut fields = fields.clone();
    fields.retain(|key, _| !VOLATILE_CREDENTIALS.contains(&key.as_str()));
    Value::Object(fields)
}

/// Starts a "connect with a code" login.
pub async fn device_start(provider: &str, http: &reqwest::Client) -> Result<DeviceStart, DebridError> {
    match provider {
        alldebrid::ID => alldebrid::device_start(http).await,
        realdebrid::ID => realdebrid::device_start(http).await,
        torbox::ID => torbox::device_start(http).await,
        _ => Err(DebridError::Other("this provider does not support connecting with a code".into())),
    }
}

pub async fn device_poll(
    provider: &str,
    http: &reqwest::Client,
    state: &serde_json::Value,
) -> Result<DevicePoll, DebridError> {
    match provider {
        alldebrid::ID => alldebrid::device_poll(http, state).await,
        realdebrid::ID => realdebrid::device_poll(http, state).await,
        torbox::ID => torbox::device_poll(http, state).await,
        _ => Err(DebridError::Other("this provider does not support connecting with a code".into())),
    }
}

fn unknown(provider: &str) -> DebridError {
    DebridError::Other(format!("unknown provider `{provider}`"))
}

/// Files of finished transfers that already are direct CDN links carry this URL fragment, so
/// that [`DebridService::unlock`] passes them through. Fragments are never sent to servers.
const DIRECT_MARKER: &str = "#rustload-direct";

pub(crate) fn mark_direct(url: &str) -> String {
    format!("{url}{DIRECT_MARKER}")
}

pub(crate) fn strip_direct(link: &str) -> Option<&str> {
    link.strip_suffix(DIRECT_MARKER)
}

#[derive(Debug, thiserror::Error)]
pub enum DebridError {
    /// Bad/expired credentials or banned account.
    #[error("authentication failed: {0}")]
    Auth(String),
    /// The provider does not handle this host.
    #[error("host not supported")]
    Unsupported,
    /// The file is gone, private or password protected: no account will do better.
    #[error("{0}")]
    LinkDown(String),
    /// Account quota or premium requirement: another account may work.
    #[error("{0}")]
    Quota(String),
    /// Temporary failure (maintenance, rate limit, network).
    #[error("{0}")]
    Temporary(String),
    #[error("{0}")]
    Other(String),
}

impl From<reqwest::Error> for DebridError {
    fn from(error: reqwest::Error) -> Self {
        // The URL may contain credentials (some APIs take the key as a query parameter).
        Self::Temporary(format!("network error: {}", error.without_url()))
    }
}

#[derive(Debug, Clone)]
pub struct AccountStatus {
    pub username: Option<String>,
    pub premium: bool,
    pub premium_until: Option<i64>,
    pub traffic_left: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct Unlocked {
    pub url: String,
    pub filename: Option<String>,
    pub size: Option<u64>,
    /// Digest of the file, when the provider publishes one (checked after the download).
    pub hash: Option<FileHash>,
}

#[derive(Debug, Clone)]
pub struct LinkInfo {
    pub filename: Option<String>,
    pub size: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransferState {
    Queued,
    Downloading,
    Processing,
    Ready,
    Error,
}

#[derive(Debug, Clone)]
pub struct TransferStatus {
    pub state: TransferState,
    /// Provider wording, shown as is in the UI.
    pub status_text: String,
    pub name: Option<String>,
    pub size: Option<u64>,
    pub progress: f64,
    pub speed: u64,
    pub seeders: Option<u32>,
}

/// A file of a finished remote transfer. `link` is passed to [`DebridService::unlock`] of the same
/// account when the download starts.
#[derive(Debug, Clone)]
pub struct RemoteFile {
    /// Relative path inside the transfer, `/`-separated, file name included.
    pub path: String,
    pub size: Option<u64>,
    pub link: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DeviceStart {
    pub user_code: String,
    pub verification_url: String,
    pub expires_in: u64,
    /// Provider specific data needed to poll.
    #[serde(skip)]
    pub state: serde_json::Value,
}

pub enum DevicePoll {
    Pending,
    Expired,
    /// Credentials JSON to store on the new account.
    Activated(String),
}

#[async_trait]
pub trait DebridService: Send + Sync {
    fn info(&self) -> ProviderInfo;
    async fn account_status(&self) -> Result<AccountStatus, DebridError>;
    async fn supported_hosts(&self) -> Result<HostMatcher, DebridError>;
    async fn unlock(&self, link: &str, password: Option<&str>) -> Result<Unlocked, DebridError>;
    /// File name and size without generating a download link (availability check).
    async fn link_info(&self, _link: &str) -> Result<Option<LinkInfo>, DebridError> {
        Ok(None)
    }
    // Magnets and torrents: only called on providers whose `ProviderInfo::magnets` is true.
    async fn add_magnet(&self, _magnet: &str) -> Result<String, DebridError> {
        Err(DebridError::Unsupported)
    }
    async fn add_torrent(&self, _file_name: &str, _data: Vec<u8>) -> Result<String, DebridError> {
        Err(DebridError::Unsupported)
    }
    async fn transfer_status(&self, _id: &str) -> Result<TransferStatus, DebridError> {
        Err(DebridError::Unsupported)
    }
    async fn transfer_files(&self, _id: &str) -> Result<Vec<RemoteFile>, DebridError> {
        Err(DebridError::Unsupported)
    }
    async fn delete_transfer(&self, _id: &str) -> Result<(), DebridError> {
        Err(DebridError::Unsupported)
    }
    /// New credentials to persist after a token refresh, if any.
    fn take_updated_credentials(&self) -> Option<String> {
        None
    }
}

/// Decides locally whether a URL belongs to a host supported by a provider. Every host a provider
/// lists counts, whatever the health it reports: that is a snapshot, while the list is cached for
/// hours, and a host wrongly left out would be downloaded directly (its web page) instead. The
/// unlock tells whether the host works, and the next account or the built-in plugin is tried if
/// not.
#[derive(Debug, Default)]
pub struct HostMatcher {
    domains: HashSet<String>,
    patterns: Vec<Regex>,
}

impl HostMatcher {
    /// A matcher for a fixed list of domains (and their sub-domains).
    pub fn from_domains(domains: &[&str]) -> Self {
        let mut matcher = Self::default();
        domains.iter().for_each(|domain| matcher.add_domain(domain));
        matcher
    }

    pub fn add_domain(&mut self, domain: &str) {
        let domain = domain.trim().trim_start_matches("www.").to_ascii_lowercase();
        if !domain.is_empty() {
            self.domains.insert(domain);
        }
    }

    /// Adds a provider regex; JavaScript-style `/.../i` delimiters are accepted. Invalid patterns
    /// are ignored (domains usually cover the host anyway).
    pub fn add_pattern(&mut self, pattern: &str) {
        let trimmed = pattern.trim();
        let body =
            trimmed.strip_prefix('/').and_then(|p| p.rsplit_once('/').map(|(body, _flags)| body)).unwrap_or(trimmed);
        if let Ok(regex) = Regex::new(&format!("(?i){body}")) {
            self.patterns.push(regex);
        }
    }

    pub fn matches(&self, url: &str) -> bool {
        // `cdn.example.com` matches `example.com`.
        crate::util::url_host(url).is_some_and(|host| parent_domains(&host).any(|d| self.domains.contains(d)))
            || self.patterns.iter().any(|p| p.is_match(url))
    }
}

/// Rate limits and server errors: worth trying again later.
pub(crate) fn temporary_status(status: u16) -> bool {
    status == 429 || status >= 500
}

/// `Temporary` error for an HTTP status worth trying again later, `None` for other statuses.
pub(crate) fn http_failure(name: &str, status: impl Into<u16> + Display + Copy) -> Option<DebridError> {
    temporary_status(status.into()).then(|| DebridError::Temporary(format!("{name} answered HTTP {status}")))
}

/// Integer sent either as a JSON number or as a string (some APIs mix both).
pub(crate) fn lenient_i64(value: &serde_json::Value) -> Option<i64> {
    match value {
        serde_json::Value::Number(number) => number.as_i64().or_else(|| number.as_f64().map(|f| f as i64)),
        serde_json::Value::String(text) => text.trim().parse().ok(),
        _ => None,
    }
}

/// Polls `check` every `interval` until it returns `Some`, or fails after `timeout`.
pub(crate) async fn poll_until<T, F, Fut>(interval: Duration, timeout: Duration, mut check: F) -> Result<T, DebridError>
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<Option<T>, DebridError>>,
{
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        if let Some(value) = check().await? {
            return Ok(value);
        }
        if tokio::time::Instant::now() + interval > deadline {
            return Err(DebridError::Temporary("the provider took too long to prepare the link".into()));
        }
        tokio::time::sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_domains_and_subdomains() {
        let mut matcher = HostMatcher::default();
        matcher.add_domain("1fichier.com");
        assert!(matcher.matches("https://1fichier.com/?abc"));
        assert!(matcher.matches("https://cdn.1fichier.com/file"));
        assert!(!matcher.matches("https://example.com/1fichier.com"));
    }

    #[test]
    fn matches_fixed_domain_lists() {
        let matcher = HostMatcher::from_domains(&["rapidgator.net", "rg.to"]);
        assert!(matcher.matches("https://rg.to/file/abc"));
        assert!(matcher.matches("https://www.rapidgator.net/file/abc"));
        assert!(!matcher.matches("https://rapidgator.network/file/abc"));
        assert!(!matcher.matches("https://example.com/?rg.to"));
    }

    #[test]
    fn reads_lenient_integers() {
        assert_eq!(lenient_i64(&serde_json::json!(42)), Some(42));
        assert_eq!(lenient_i64(&serde_json::json!("4398046511104")), Some(4_398_046_511_104));
        assert_eq!(lenient_i64(&serde_json::json!(null)), None);
        assert_eq!(lenient_i64(&serde_json::json!("inf")), None);
    }

    #[test]
    fn lists_providers_with_unique_ids() {
        let providers = providers();
        let mut ids: Vec<&str> = providers.iter().map(|p| p.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), providers.len());
        let hosters = providers.iter().filter(|p| p.kind == ProviderKind::Hoster);
        assert!(hosters.clone().count() >= 2);
        assert!(hosters.into_iter().all(|p| !p.magnets && !p.device_auth));
    }

    #[test]
    fn credentials_follow_the_provider_auth_kind() {
        assert_eq!(api_key_credentials("1fichier", " key ").unwrap(), r#"{"api_key":"key"}"#);
        assert!(api_key_credentials("realdebrid", "key").is_ok());
        assert!(api_key_credentials("nope", "key").is_err());
        assert!(api_key_credentials("rapidgator", "key").is_err());
        let login = login_credentials("rapidgator", " me@example.com ", "secret").unwrap();
        assert_eq!(login_username(&login).as_deref(), Some("me@example.com"));
        assert!(login_credentials("1fichier", "me", "secret").is_err());
        assert!(login_credentials("nope", "me", "secret").is_err());
    }

    #[test]
    fn exports_credentials_without_session_tokens() {
        assert_eq!(
            without_session(r#"{"username":"me","password":"pw","token":"t"}"#),
            serde_json::json!({ "username": "me", "password": "pw" })
        );
        // OAuth tokens are what the account needs to work: kept.
        let oauth = r#"{"client_id":"c","client_secret":"s","access_token":"a","refresh_token":"r","expires_at":1}"#;
        assert_eq!(without_session(oauth), serde_json::from_str::<Value>(oauth).unwrap());
        assert_eq!(without_session(r#"{"api_key":"k"}"#), serde_json::json!({ "api_key": "k" }));
        assert_eq!(without_session("not json"), serde_json::json!("not json"));
    }

    #[test]
    fn masks_the_main_secret() {
        assert_eq!(masked_secret(r#"{"api_key":"abcdefghijkl"}"#), "abcd••••ijkl");
        assert_eq!(masked_secret(r#"{"access_token":"abcdefghijkl","refresh_token":"r"}"#), "abcd••••ijkl");
        assert_eq!(masked_secret(r#"{"username":"me@example.com","password":"a long password"}"#), "••••");
    }

    #[test]
    fn identifies_accounts_whatever_their_tokens() {
        let identity = |raw: &str| account_identity(&serde_json::from_str(raw).unwrap());
        assert_eq!(identity(r#"{"api_key":" k ","extra":1}"#), serde_json::json!({ "api_key": "k" }));
        assert_eq!(
            identity(r#"{"username":" Me@X.com","password":"p","token":"t"}"#),
            identity(r#"{"username":"me@x.com"}"#)
        );
        assert_eq!(
            identity(r#"{"client_id":"C","client_secret":"s","access_token":"a","refresh_token":"r","expires_at":1}"#),
            serde_json::json!({ "client_id": "C" })
        );
        assert_eq!(identity(r#"{"other":1,"expires_at":2}"#), serde_json::json!({ "other": 1 }));
    }

    #[test]
    fn classifies_http_failures() {
        assert!(matches!(
            http_failure("AllDebrid", reqwest::StatusCode::SERVICE_UNAVAILABLE),
            Some(DebridError::Temporary(m)) if m == "AllDebrid answered HTTP 503 Service Unavailable"
        ));
        assert!(
            matches!(http_failure("Keep2Share", 429u16), Some(DebridError::Temporary(m)) if m == "Keep2Share answered HTTP 429")
        );
        assert!(http_failure("AllDebrid", reqwest::StatusCode::NOT_FOUND).is_none());
    }

    #[test]
    fn matches_js_style_patterns() {
        let mut matcher = HostMatcher::default();
        matcher.add_pattern(r"/https?:\/\/(?:www\.)?rapidgator\.net\/file\//i");
        assert!(matcher.matches("https://RAPIDGATOR.net/file/123"));
        assert!(!matcher.matches("https://rapidgator.net/folder/1"));
    }
}
