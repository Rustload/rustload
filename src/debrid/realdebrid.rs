//! Real-Debrid — <https://api.real-debrid.com/>
//!
//! Accounts use either a private API token or the OAuth device flow with Real-Debrid's public
//! client for open-source apps; OAuth tokens are refreshed transparently.

use std::sync::Mutex;

use async_trait::async_trait;
use reqwest::{RequestBuilder, StatusCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::Value;

use super::{
    AccountStatus, AuthKind, DebridError, DebridService, DevicePoll, DeviceStart, HostMatcher, LinkInfo, ProviderInfo,
    ProviderKind, RemoteFile, TransferState, TransferStatus, Unlocked, temporary_status,
};
use crate::util::now;

pub const ID: &str = "realdebrid";
const API: &str = "https://api.real-debrid.com/rest/1.0";
const OAUTH: &str = "https://api.real-debrid.com/oauth/v2";
/// Real-Debrid's public client id for open-source applications.
const OPEN_SOURCE_CLIENT_ID: &str = "X245A4XAIBGVM";
const DEVICE_GRANT: &str = "http://oauth.net/grant_type/device/1.0";

pub const INFO: ProviderInfo = ProviderInfo {
    id: ID,
    name: "Real-Debrid",
    website: "https://real-debrid.com",
    api_key_url: "https://real-debrid.com/apitoken",
    device_auth: true,
    magnets: true,
    kind: ProviderKind::Debrid,
    auth: AuthKind::ApiKey,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum Credentials {
    OAuth { client_id: String, client_secret: String, access_token: String, refresh_token: String, expires_at: i64 },
    ApiKey { api_key: String },
}

impl Credentials {
    fn token(&self) -> &str {
        match self {
            Self::OAuth { access_token, .. } => access_token,
            Self::ApiKey { api_key } => api_key,
        }
    }
}

pub struct RealDebrid {
    http: reqwest::Client,
    credentials: Mutex<Credentials>,
    updated: Mutex<Option<String>>,
}

impl RealDebrid {
    pub fn from_credentials(raw: &str, http: reqwest::Client) -> Result<Self, DebridError> {
        let credentials = serde_json::from_str(raw).map_err(|_| DebridError::Auth("invalid credentials".into()))?;
        Ok(Self { http, credentials: Mutex::new(credentials), updated: Mutex::new(None) })
    }

    fn current(&self) -> Credentials {
        self.credentials.lock().expect("poisoned").clone()
    }

    /// Sends an authenticated request, refreshing an expired OAuth token once.
    async fn send(&self, request: RequestBuilder) -> Result<Option<Value>, DebridError> {
        if let Credentials::OAuth { expires_at, .. } = self.current()
            && expires_at <= now() + 60
        {
            self.refresh().await?;
        }
        let retry = request.try_clone();
        let response = request.bearer_auth(self.current().token()).send().await?;
        if response.status() == StatusCode::UNAUTHORIZED
            && let (Credentials::OAuth { .. }, Some(retry)) = (self.current(), retry)
        {
            self.refresh().await?;
            return parse(retry.bearer_auth(self.current().token()).send().await?).await;
        }
        parse(response).await
    }

    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T, DebridError> {
        let value = self.send(self.http.get(format!("{API}{path}"))).await?;
        decode(value.unwrap_or(Value::Null))
    }

    async fn post<T: DeserializeOwned>(&self, path: &str, form: &[(&str, &str)]) -> Result<T, DebridError> {
        let value = self.send(self.http.post(format!("{API}{path}")).form(form)).await?;
        decode(value.unwrap_or(Value::Null))
    }

    async fn refresh(&self) -> Result<(), DebridError> {
        let Credentials::OAuth { client_id, client_secret, refresh_token, .. } = self.current() else { return Ok(()) };
        let token: TokenResponse = token_request(&self.http, &client_id, &client_secret, &refresh_token).await?;
        let refreshed = Credentials::OAuth {
            client_id,
            client_secret,
            access_token: token.access_token,
            refresh_token: token.refresh_token,
            expires_at: now() + token.expires_in,
        };
        *self.updated.lock().expect("poisoned") = Some(serde_json::to_string(&refreshed).expect("serializable"));
        *self.credentials.lock().expect("poisoned") = refreshed;
        Ok(())
    }
}

/// Maps a response: `None` for empty bodies (201/204), errors from `error_code`.
async fn parse(response: reqwest::Response) -> Result<Option<Value>, DebridError> {
    let status = response.status();
    let text = response.text().await.map_err(DebridError::from)?;
    if status.is_success() {
        return Ok((!text.trim().is_empty()).then(|| serde_json::from_str(&text).unwrap_or(Value::Null)));
    }
    let body: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
    Err(map_error(status, &body))
}

fn decode<T: DeserializeOwned>(value: Value) -> Result<T, DebridError> {
    serde_json::from_value(value).map_err(|e| DebridError::Other(format!("unexpected Real-Debrid response: {e}")))
}

fn map_error(status: StatusCode, body: &Value) -> DebridError {
    let code = body["error_code"].as_i64();
    let message = body["error"].as_str().unwrap_or("unknown error").replace('_', " ");
    match code {
        Some(8 | 12 | 13 | 14 | 15) => DebridError::Auth(message),
        Some(16) => DebridError::Unsupported,
        Some(7 | 24 | 26 | 28 | 29 | 30 | 35) => DebridError::LinkDown(message),
        Some(9 | 18 | 20 | 23 | 36) => DebridError::Quota(message),
        Some(-1 | 5 | 6 | 17 | 19 | 21 | 25 | 27 | 34) => DebridError::Temporary(message),
        Some(22) => DebridError::Other("Real-Debrid refuses this IP address (VPN or datacenter)".into()),
        _ if status == StatusCode::UNAUTHORIZED => DebridError::Auth(message),
        _ if temporary_status(status.as_u16()) => DebridError::Temporary(message),
        _ => DebridError::Other(message),
    }
}

#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    refresh_token: String,
    expires_in: i64,
}

async fn token_request<T: DeserializeOwned>(
    http: &reqwest::Client,
    client_id: &str,
    client_secret: &str,
    code: &str,
) -> Result<T, DebridError> {
    let response = http
        .post(format!("{OAUTH}/token"))
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret),
            ("code", code),
            ("grant_type", DEVICE_GRANT),
        ])
        .send()
        .await?;
    let value = parse(response).await.map_err(|e| match e {
        DebridError::Other(message) => DebridError::Auth(message),
        other => other,
    })?;
    decode(value.unwrap_or(Value::Null))
}

#[derive(Deserialize)]
struct User {
    username: Option<String>,
    #[serde(rename = "type")]
    kind: Option<String>,
    #[serde(default)]
    premium: i64,
}

#[derive(Deserialize)]
struct Unrestricted {
    download: String,
    filename: Option<String>,
    filesize: Option<u64>,
}

#[derive(Deserialize)]
struct TorrentInfo {
    status: String,
    filename: Option<String>,
    #[serde(default)]
    bytes: u64,
    #[serde(default)]
    progress: f64,
    #[serde(default)]
    speed: u64,
    seeders: Option<u32>,
    #[serde(default)]
    files: Vec<TorrentFile>,
    #[serde(default)]
    links: Vec<String>,
}

#[derive(Deserialize)]
struct TorrentFile {
    id: u64,
    path: String,
    bytes: u64,
    selected: u8,
}

#[async_trait]
impl DebridService for RealDebrid {
    fn info(&self) -> ProviderInfo {
        INFO
    }

    async fn account_status(&self) -> Result<AccountStatus, DebridError> {
        let user: User = self.get("/user").await?;
        let premium = user.kind.as_deref() == Some("premium") && user.premium > 0;
        Ok(AccountStatus {
            username: user.username,
            premium,
            premium_until: premium.then(|| now() + user.premium),
            traffic_left: None,
        })
    }

    async fn supported_hosts(&self) -> Result<HostMatcher, DebridError> {
        let domains: Vec<String> = self.get("/hosts/domains").await?;
        let patterns: Vec<String> = self.get("/hosts/regex").await.unwrap_or_default();
        let mut matcher = HostMatcher::default();
        domains.iter().for_each(|d| matcher.add_domain(d));
        patterns.iter().for_each(|p| matcher.add_pattern(p));
        Ok(matcher)
    }

    async fn unlock(&self, link: &str, password: Option<&str>) -> Result<Unlocked, DebridError> {
        let mut form = vec![("link", link)];
        if let Some(password) = password {
            form.push(("password", password));
        }
        let data: Unrestricted = self.post("/unrestrict/link", &form).await?;
        Ok(Unlocked { url: data.download, filename: data.filename, size: data.filesize, hash: None })
    }

    async fn link_info(&self, link: &str) -> Result<Option<LinkInfo>, DebridError> {
        let data: Value = self.post("/unrestrict/check", &[("link", link)]).await?;
        Ok(Some(LinkInfo { filename: data["filename"].as_str().map(str::to_owned), size: data["filesize"].as_u64() }))
    }

    async fn add_magnet(&self, magnet: &str) -> Result<String, DebridError> {
        let data: Value = self.post("/torrents/addMagnet", &[("magnet", magnet)]).await?;
        data["id"].as_str().map(str::to_owned).ok_or_else(|| DebridError::Other("no torrent id returned".into()))
    }

    async fn add_torrent(&self, _file_name: &str, data: Vec<u8>) -> Result<String, DebridError> {
        // The .torrent is sent as the raw request body, not as multipart.
        let value = self.send(self.http.put(format!("{API}/torrents/addTorrent")).body(data)).await?;
        value
            .and_then(|v| v["id"].as_str().map(str::to_owned))
            .ok_or_else(|| DebridError::Other("no torrent id returned".into()))
    }

    async fn transfer_status(&self, id: &str) -> Result<TransferStatus, DebridError> {
        let info: TorrentInfo = self.get(&format!("/torrents/info/{id}")).await?;
        let state = match info.status.as_str() {
            "waiting_files_selection" => {
                // Mandatory step: without it the torrent never starts.
                self.post::<Value>(&format!("/torrents/selectFiles/{id}"), &[("files", "all")]).await?;
                TransferState::Queued
            }
            "magnet_conversion" | "queued" => TransferState::Queued,
            "downloading" => TransferState::Downloading,
            "compressing" | "uploading" => TransferState::Processing,
            "downloaded" => TransferState::Ready,
            _ => TransferState::Error,
        };
        Ok(TransferStatus {
            state,
            status_text: info.status.replace('_', " "),
            name: info.filename,
            size: (info.bytes > 0).then_some(info.bytes),
            progress: (info.progress / 100.0).clamp(0.0, 1.0),
            speed: info.speed,
            seeders: info.seeders,
        })
    }

    async fn transfer_files(&self, id: &str) -> Result<Vec<RemoteFile>, DebridError> {
        let info: TorrentInfo = self.get(&format!("/torrents/info/{id}")).await?;
        let mut selected: Vec<&TorrentFile> = info.files.iter().filter(|f| f.selected == 1).collect();
        selected.sort_by_key(|f| f.id);
        // One link per selected file, in file id order — unless Real-Debrid packed them into an
        // archive, in which case the links do not map to files.
        if selected.len() == info.links.len() {
            return Ok(selected
                .iter()
                .zip(&info.links)
                .map(|(file, link)| RemoteFile {
                    path: file.path.trim_start_matches('/').to_owned(),
                    size: Some(file.bytes),
                    link: link.clone(),
                })
                .collect());
        }
        let name = info.filename.unwrap_or_else(|| "torrent".into());
        Ok(info
            .links
            .iter()
            .enumerate()
            .map(|(index, link)| RemoteFile { path: format!("{name}.{}", index + 1), size: None, link: link.clone() })
            .collect())
    }

    async fn delete_transfer(&self, id: &str) -> Result<(), DebridError> {
        self.send(self.http.delete(format!("{API}/torrents/delete/{id}"))).await?;
        Ok(())
    }

    fn take_updated_credentials(&self) -> Option<String> {
        self.updated.lock().expect("poisoned").take()
    }
}

pub async fn device_start(http: &reqwest::Client) -> Result<DeviceStart, DebridError> {
    let response = http
        .get(format!("{OAUTH}/device/code"))
        .query(&[("client_id", OPEN_SOURCE_CLIENT_ID), ("new_credentials", "yes")])
        .send()
        .await?;
    let data = parse(response).await?.unwrap_or(Value::Null);
    Ok(DeviceStart {
        user_code: data["user_code"].as_str().unwrap_or_default().to_owned(),
        verification_url: data["direct_verification_url"]
            .as_str()
            .or(data["verification_url"].as_str())
            .unwrap_or("https://real-debrid.com/device")
            .to_owned(),
        expires_in: data["expires_in"].as_u64().unwrap_or(900),
        state: serde_json::json!({ "device_code": data["device_code"] }),
    })
}

pub async fn device_poll(http: &reqwest::Client, state: &Value) -> Result<DevicePoll, DebridError> {
    let device_code = state["device_code"].as_str().unwrap_or_default();
    let response = http
        .get(format!("{OAUTH}/device/credentials"))
        .query(&[("client_id", OPEN_SOURCE_CLIENT_ID), ("code", device_code)])
        .send()
        .await?;
    // 403 while the user has not approved yet.
    if response.status() == StatusCode::FORBIDDEN {
        return Ok(DevicePoll::Pending);
    }
    let data = parse(response).await?.unwrap_or(Value::Null);
    let (Some(client_id), Some(client_secret)) = (data["client_id"].as_str(), data["client_secret"].as_str()) else {
        return Ok(DevicePoll::Pending);
    };
    let token: TokenResponse = token_request(http, client_id, client_secret, device_code).await?;
    let credentials = Credentials::OAuth {
        client_id: client_id.to_owned(),
        client_secret: client_secret.to_owned(),
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_at: now() + token.expires_in,
    };
    Ok(DevicePoll::Activated(serde_json::to_string(&credentials).expect("serializable")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_error_codes() {
        let err =
            |code: i64| map_error(StatusCode::BAD_REQUEST, &serde_json::json!({ "error": "x", "error_code": code }));
        assert!(matches!(err(8), DebridError::Auth(_)));
        assert!(matches!(err(16), DebridError::Unsupported));
        assert!(matches!(err(24), DebridError::LinkDown(_)));
        assert!(matches!(err(23), DebridError::Quota(_)));
        assert!(matches!(err(34), DebridError::Temporary(_)));
    }

    #[test]
    fn reads_both_credential_kinds() {
        let key: Credentials = serde_json::from_str(r#"{"api_key":"abc"}"#).unwrap();
        assert_eq!(key.token(), "abc");
        let oauth: Credentials = serde_json::from_str(
            r#"{"client_id":"a","client_secret":"b","access_token":"t","refresh_token":"r","expires_at":1}"#,
        )
        .unwrap();
        assert_eq!(oauth.token(), "t");
    }
}
