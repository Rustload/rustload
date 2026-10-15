//! TorBox — <https://api-docs.torbox.app/>
//!
//! Hoster links are "web downloads": TorBox fetches the file first, then serves it. File links of
//! finished transfers are generated on demand, so they are stored as `torbox://` references.

use std::time::Duration;

use async_trait::async_trait;
use chrono::DateTime;
use reqwest::{RequestBuilder, multipart};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use super::{
    AccountStatus, AuthKind, DebridError, DebridService, DevicePoll, DeviceStart, HostMatcher, ProviderInfo,
    ProviderKind, RemoteFile, TransferState, TransferStatus, Unlocked, api_key, poll_until, temporary_status,
};

pub const ID: &str = "torbox";
const API: &str = "https://api.torbox.app/v1/api";
const SCHEME: &str = "torbox://";

pub const INFO: ProviderInfo = ProviderInfo {
    id: ID,
    name: "TorBox",
    website: "https://torbox.app",
    api_key_url: "https://torbox.app/settings",
    device_auth: true,
    magnets: true,
    kind: ProviderKind::Debrid,
    auth: AuthKind::ApiKey,
};

pub struct TorBox {
    http: reqwest::Client,
    api_key: String,
}

impl TorBox {
    pub fn from_credentials(raw: &str, http: reqwest::Client) -> Result<Self, DebridError> {
        Ok(Self { http, api_key: api_key::decode(raw)? })
    }

    async fn send<T: DeserializeOwned>(&self, request: RequestBuilder) -> Result<T, DebridError> {
        envelope(request.bearer_auth(&self.api_key).send().await?).await
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T, DebridError> {
        self.send(self.http.get(format!("{API}{path}")).query(query)).await
    }

    /// Download URL of one file; `kind` is `torrents` or `webdl`.
    async fn request_link(&self, kind: &str, id: &str, file_id: &str) -> Result<String, DebridError> {
        let id_param = if kind == "torrents" { "torrent_id" } else { "web_id" };
        // These endpoints take the key as a query parameter.
        let request = self.http.get(format!("{API}/{kind}/requestdl")).query(&[
            ("token", self.api_key.as_str()),
            (id_param, id),
            ("file_id", file_id),
        ]);
        envelope(request.send().await?).await
    }
}

/// Unwraps `{ success, error, detail, data }`.
async fn envelope<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, DebridError> {
    let status = response.status();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    if body["success"] == true {
        return serde_json::from_value(body["data"].clone())
            .map_err(|e| DebridError::Other(format!("unexpected TorBox response: {e}")));
    }
    if status.as_u16() == 401 {
        return Err(DebridError::Auth("not authenticated".into()));
    }
    Err(map_error(status.as_u16(), &body))
}

fn map_error(status: u16, body: &Value) -> DebridError {
    let code = body["error"].as_str().unwrap_or("UNKNOWN_ERROR");
    let message = body["detail"].as_str().map(str::to_owned).unwrap_or_else(|| code.to_owned());
    match code {
        "NO_AUTH" | "BAD_TOKEN" => DebridError::Auth(message),
        "AUTH_ERROR" if status == 403 => DebridError::Auth(message),
        "UNSUPPORTED_SITE" => DebridError::Unsupported,
        "LINK_OFFLINE" | "INVALID_LINK" | "ITEM_NOT_FOUND" | "BOZO_TORRENT" | "BOZO_FILE" | "TOO_MUCH_DATA" => {
            DebridError::LinkDown(message)
        }
        "PLAN_RESTRICTED_FEATURE" | "DOWNLOAD_TOO_LARGE" | "MONTHLY_LIMIT" | "COOLDOWN_LIMIT" | "DIFF_ISSUE" => {
            DebridError::Quota(message)
        }
        "ACTIVE_LIMIT" | "TEMPORARILY_DISABLED" => DebridError::Temporary(message),
        code if code.ends_with("_ERROR") || temporary_status(status) => DebridError::Temporary(message),
        _ => DebridError::Other(message),
    }
}

/// `torbox://torrents/12/3` → (`torrents`, `12`, `3`).
fn parse_reference(link: &str) -> Option<(&str, &str, &str)> {
    let mut parts = link.strip_prefix(SCHEME)?.split('/');
    Some((parts.next()?, parts.next()?, parts.next()?))
}

fn map_state(item: &Value) -> (TransferState, String) {
    let state = item["download_state"].as_str().unwrap_or("unknown").to_owned();
    if item["download_present"] == true {
        return (TransferState::Ready, state);
    }
    let lower = state.to_ascii_lowercase();
    let normalized = if lower.starts_with("downloading") || lower.starts_with("stalled") {
        TransferState::Downloading
    } else if lower.starts_with("error") || lower.starts_with("failed") || lower == "missing" {
        TransferState::Error
    } else if matches!(lower.as_str(), "completed" | "uploading" | "processing" | "cached") {
        TransferState::Processing
    } else {
        TransferState::Queued
    };
    (normalized, state)
}

#[async_trait]
impl DebridService for TorBox {
    fn info(&self) -> ProviderInfo {
        INFO
    }

    async fn account_status(&self) -> Result<AccountStatus, DebridError> {
        let user: Value = self.get("/user/me", &[("settings", "false")]).await?;
        let premium_until = user["premium_expires_at"]
            .as_str()
            .and_then(|date| DateTime::parse_from_rfc3339(date).ok())
            .map(|date| date.timestamp())
            .filter(|t| *t > crate::util::now());
        Ok(AccountStatus {
            username: user["email"].as_str().map(str::to_owned),
            premium: user["plan"].as_i64().unwrap_or(0) > 0 && premium_until.is_some(),
            premium_until,
            traffic_left: None,
        })
    }

    async fn supported_hosts(&self) -> Result<HostMatcher, DebridError> {
        let hosts: Vec<Value> = self.get("/webdl/hosters", &[]).await?;
        let mut matcher = HostMatcher::default();
        for host in hosts.iter().filter(|h| h["status"] == true) {
            for domain in host["domains"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                matcher.add_domain(domain);
            }
            if let Some(pattern) = host["regex"].as_str() {
                matcher.add_pattern(pattern);
            }
        }
        Ok(matcher)
    }

    async fn unlock(&self, link: &str, password: Option<&str>) -> Result<Unlocked, DebridError> {
        if let Some((kind, id, file_id)) = parse_reference(link) {
            let url = self.request_link(kind, id, file_id).await?;
            return Ok(Unlocked { url, filename: None, size: None, hash: None });
        }
        // Hoster link: TorBox downloads it first (a "web download").
        let mut form = vec![("link", link)];
        if let Some(password) = password {
            form.push(("password", password));
        }
        let created: Value = self.send(self.http.post(format!("{API}/webdl/createwebdownload")).form(&form)).await?;
        let id = created["webdownload_id"]
            .as_u64()
            .ok_or_else(|| DebridError::Other("no web download id".into()))?
            .to_string();
        let item = poll_until(Duration::from_secs(5), Duration::from_secs(30 * 60), || async {
            let item: Value = self.get("/webdl/mylist", &[("id", &id), ("bypass_cache", "true")]).await?;
            match map_state(&item).0 {
                TransferState::Ready => Ok(Some(item)),
                TransferState::Error => {
                    Err(DebridError::LinkDown(item["error"].as_str().unwrap_or("TorBox failed").to_owned()))
                }
                _ => Ok(None),
            }
        })
        .await?;
        let file = &item["files"][0];
        let file_id = file["id"].as_u64().map(|i| i.to_string()).unwrap_or_else(|| "0".into());
        let url = self.request_link("webdl", &id, &file_id).await?;
        Ok(Unlocked {
            url,
            filename: file["short_name"].as_str().map(str::to_owned),
            size: file["size"].as_u64(),
            hash: None,
        })
    }

    async fn add_magnet(&self, magnet: &str) -> Result<String, DebridError> {
        let form = multipart::Form::new().text("magnet", magnet.to_owned());
        let created: Value = self.send(self.http.post(format!("{API}/torrents/createtorrent")).multipart(form)).await?;
        created["torrent_id"].as_u64().map(|i| i.to_string()).ok_or_else(|| DebridError::Other("no torrent id".into()))
    }

    async fn add_torrent(&self, file_name: &str, data: Vec<u8>) -> Result<String, DebridError> {
        let form = multipart::Form::new().part("file", multipart::Part::bytes(data).file_name(file_name.to_owned()));
        let created: Value = self.send(self.http.post(format!("{API}/torrents/createtorrent")).multipart(form)).await?;
        created["torrent_id"].as_u64().map(|i| i.to_string()).ok_or_else(|| DebridError::Other("no torrent id".into()))
    }

    async fn transfer_status(&self, id: &str) -> Result<TransferStatus, DebridError> {
        let item: Value = self.get("/torrents/mylist", &[("id", id), ("bypass_cache", "true")]).await?;
        let (state, text) = map_state(&item);
        Ok(TransferStatus {
            state,
            status_text: text,
            name: item["name"].as_str().map(str::to_owned),
            size: item["size"].as_u64(),
            progress: item["progress"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0),
            speed: item["download_speed"].as_u64().unwrap_or(0),
            seeders: item["seeds"].as_u64().map(|s| s as u32),
        })
    }

    async fn transfer_files(&self, id: &str) -> Result<Vec<RemoteFile>, DebridError> {
        let item: Value = self.get("/torrents/mylist", &[("id", id), ("bypass_cache", "true")]).await?;
        Ok(item["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|file| {
                Some(RemoteFile {
                    path: file["name"].as_str()?.to_owned(),
                    size: file["size"].as_u64(),
                    link: format!("{SCHEME}torrents/{id}/{}", file["id"].as_u64()?),
                })
            })
            .collect())
    }

    async fn delete_transfer(&self, id: &str) -> Result<(), DebridError> {
        let torrent_id: u64 = id.parse().map_err(|_| DebridError::Other("invalid torrent id".into()))?;
        let body = json!({ "torrent_id": torrent_id, "operation": "delete" });
        let _: Value = self.send(self.http.post(format!("{API}/torrents/controltorrent")).json(&body)).await?;
        Ok(())
    }
}

pub async fn device_start(http: &reqwest::Client) -> Result<DeviceStart, DebridError> {
    let response = http.get(format!("{API}/user/auth/device/start")).query(&[("app", "Rustload")]).send().await?;
    let data: Value = envelope(response).await?;
    Ok(DeviceStart {
        user_code: data["code"].as_str().unwrap_or_default().to_owned(),
        verification_url: data["verification_url_complete"]
            .as_str()
            .or(data["verification_url"].as_str())
            .unwrap_or("https://tor.box/link")
            .to_owned(),
        expires_in: data["expires_in"].as_u64().unwrap_or(600),
        state: json!({ "device_code": data["device_code"] }),
    })
}

pub async fn device_poll(http: &reqwest::Client, state: &Value) -> Result<DevicePoll, DebridError> {
    let response = http
        .post(format!("{API}/user/auth/device/token"))
        .json(&json!({ "device_code": state["device_code"] }))
        .send()
        .await?;
    let status = response.status().as_u16();
    let body: Value = response.json().await.unwrap_or(Value::Null);
    match (body["success"].as_bool(), body["error"].as_str()) {
        (Some(true), _) => match body["data"]["access_token"].as_str() {
            Some(key) => Ok(DevicePoll::Activated(api_key::encode(key))),
            None => Ok(DevicePoll::Pending),
        },
        (_, Some("DEVICE_CODE_NOT_USED")) => Ok(DevicePoll::Pending),
        (_, Some("ITEM_NOT_FOUND")) => Ok(DevicePoll::Expired),
        _ => Err(map_error(status, &body)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_file_references() {
        assert_eq!(parse_reference("torbox://torrents/12/3"), Some(("torrents", "12", "3")));
        assert_eq!(parse_reference("https://example.com"), None);
    }

    #[test]
    fn maps_states() {
        assert_eq!(
            map_state(&json!({ "download_state": "downloading", "download_present": false })).0,
            TransferState::Downloading
        );
        assert_eq!(
            map_state(&json!({ "download_state": "completed", "download_present": true })).0,
            TransferState::Ready
        );
        assert_eq!(map_state(&json!({ "download_state": "metaDL" })).0, TransferState::Queued);
        assert_eq!(map_state(&json!({ "download_state": "failed (no peers)" })).0, TransferState::Error);
    }
}
