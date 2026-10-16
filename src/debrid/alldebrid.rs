//! AllDebrid — <https://docs.alldebrid.com/>

use std::time::Duration;

use async_trait::async_trait;
use reqwest::{Method, multipart};
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::Value;

use super::{
    AccountStatus, AuthKind, DebridError, DebridService, DevicePoll, DeviceStart, HostMatcher, LinkInfo, ProviderInfo,
    ProviderKind, RemoteFile, TransferState, TransferStatus, Unlocked, api_key, http_failure, poll_until,
};
use crate::util::join_relative;

pub const ID: &str = "alldebrid";
const API: &str = "https://api.alldebrid.com";

pub const INFO: ProviderInfo = ProviderInfo {
    id: ID,
    name: "AllDebrid",
    website: "https://alldebrid.com",
    api_key_url: "https://alldebrid.com/apikeys/",
    device_auth: true,
    magnets: true,
    kind: ProviderKind::Debrid,
    auth: AuthKind::ApiKey,
};

pub struct AllDebrid {
    http: reqwest::Client,
    api_key: String,
}

impl AllDebrid {
    pub fn from_credentials(raw: &str, http: reqwest::Client) -> Result<Self, DebridError> {
        Ok(Self { http, api_key: api_key::decode(raw)? })
    }

    async fn call<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        form: &[(&str, &str)],
    ) -> Result<T, DebridError> {
        let request = self.http.request(method.clone(), format!("{API}{path}")).bearer_auth(&self.api_key);
        let request = if method == Method::GET { request.query(form) } else { request.form(form) };
        envelope(request.send().await?).await
    }
}

/// Unwraps `{ status, data, error: { code, message } }`.
async fn envelope<T: DeserializeOwned>(response: reqwest::Response) -> Result<T, DebridError> {
    if let Some(error) = http_failure("AllDebrid", response.status()) {
        return Err(error);
    }
    let body: Value = response
        .json()
        .await
        .map_err(|e| DebridError::Temporary(format!("invalid AllDebrid response: {}", e.without_url())))?;
    if body["status"] == "success" {
        return serde_json::from_value(body["data"].clone())
            .map_err(|e| DebridError::Other(format!("unexpected AllDebrid response: {e}")));
    }
    Err(map_error(&body["error"]))
}

fn map_error(error: &Value) -> DebridError {
    let code = error["code"].as_str().unwrap_or("UNKNOWN");
    let message = error["message"].as_str().unwrap_or(code).to_owned();
    match code {
        c if c.starts_with("AUTH_") => DebridError::Auth(message),
        "LINK_HOST_NOT_SUPPORTED" | "LINK_NOT_SUPPORTED" => DebridError::Unsupported,
        "LINK_DOWN" | "LINK_PASS_PROTECTED" | "LINK_IS_MISSING" => DebridError::LinkDown(message),
        "LINK_HOST_LIMIT_REACHED"
        | "MUST_BE_PREMIUM"
        | "FREE_TRIAL_LIMIT_REACHED"
        | "MAGNET_MUST_BE_PREMIUM"
        | "LINK_TOO_MANY_DOWNLOADS" => DebridError::Quota(message),
        "LINK_HOST_UNAVAILABLE"
        | "LINK_TEMPORARY_UNAVAILABLE"
        | "LINK_HOST_FULL"
        | "LINK_ERROR"
        | "MAGNET_TOO_MANY_ACTIVE"
        | "MAGNET_PROCESSING"
        | "MAINTENANCE" => DebridError::Temporary(message),
        "NO_SERVER" => DebridError::Other(format!("{message} (AllDebrid blocks VPN and datacenter IPs)")),
        _ => DebridError::Other(message),
    }
}

#[derive(Deserialize)]
struct UserData {
    user: User,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct User {
    username: Option<String>,
    #[serde(default)]
    is_premium: bool,
    #[serde(default, deserialize_with = "lenient_i64")]
    premium_until: i64,
}

/// `premiumUntil` is sometimes sent as a string.
fn lenient_i64<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<i64, D::Error> {
    Ok(super::lenient_i64(&Value::deserialize(deserializer)?).unwrap_or(0))
}

#[derive(Deserialize)]
struct UnlockData {
    #[serde(default)]
    link: String,
    filename: Option<String>,
    filesize: Option<u64>,
    id: Option<String>,
    delayed: Option<Value>,
    #[serde(default)]
    streams: Vec<Stream>,
}

#[derive(Deserialize)]
struct Stream {
    id: String,
    #[serde(default)]
    quality: Value,
    #[serde(default)]
    filesize: Option<u64>,
}

#[derive(Deserialize)]
struct DelayedData {
    status: u8,
    #[serde(default)]
    link: String,
}

#[async_trait]
impl DebridService for AllDebrid {
    fn info(&self) -> ProviderInfo {
        INFO
    }

    async fn account_status(&self) -> Result<AccountStatus, DebridError> {
        let data: UserData = self.call(Method::GET, "/v4/user", &[]).await?;
        Ok(AccountStatus {
            username: data.user.username,
            premium: data.user.is_premium,
            premium_until: (data.user.premium_until > 0).then_some(data.user.premium_until),
            traffic_left: None,
        })
    }

    async fn supported_hosts(&self) -> Result<HostMatcher, DebridError> {
        let data: Value = self.call(Method::GET, "/v4/hosts", &[]).await?;
        Ok(host_matcher(&data))
    }

    async fn unlock(&self, link: &str, password: Option<&str>) -> Result<Unlocked, DebridError> {
        let mut form = vec![("link", link)];
        if let Some(password) = password {
            form.push(("password", password));
        }
        let data: UnlockData = self.call(Method::POST, "/v4/link/unlock", &form).await?;
        let filename = data.filename.clone();
        let mut size = data.filesize;

        let url = if !data.link.is_empty() {
            data.link
        } else if let Some(delayed) = data.delayed.as_ref().and_then(value_to_string) {
            self.wait_delayed(&delayed).await?
        } else if let (Some(id), Some(best)) = (data.id.as_deref(), best_stream(&data.streams)) {
            // Streaming sites: pick the best quality, which may itself be delayed.
            size = best.filesize.or(size);
            let stream: UnlockData =
                self.call(Method::POST, "/v4/link/streaming", &[("id", id), ("stream", &best.id)]).await?;
            match stream.delayed.as_ref().and_then(value_to_string) {
                Some(delayed) if stream.link.is_empty() => self.wait_delayed(&delayed).await?,
                _ => stream.link,
            }
        } else {
            return Err(DebridError::Other("AllDebrid returned no download link".into()));
        };
        Ok(Unlocked { url, filename, size, hash: None })
    }

    async fn link_info(&self, link: &str) -> Result<Option<LinkInfo>, DebridError> {
        let data: Value = self.call(Method::POST, "/v4/link/infos", &[("link[]", link)]).await?;
        let Some(info) = data["infos"].as_array().and_then(|items| items.first()) else { return Ok(None) };
        if !info["error"].is_null() {
            return Err(map_error(&info["error"]));
        }
        Ok(Some(LinkInfo { filename: info["filename"].as_str().map(str::to_owned), size: info["size"].as_u64() }))
    }

    async fn add_magnet(&self, magnet: &str) -> Result<String, DebridError> {
        let data: Value = self.call(Method::POST, "/v4/magnet/upload", &[("magnets[]", magnet)]).await?;
        first_upload_id(&data["magnets"])
    }

    async fn add_torrent(&self, file_name: &str, bytes: Vec<u8>) -> Result<String, DebridError> {
        let part = multipart::Part::bytes(bytes)
            .file_name(file_name.to_owned())
            .mime_str("application/x-bittorrent")
            .map_err(|e| DebridError::Other(e.to_string()))?;
        let response = self
            .http
            .post(format!("{API}/v4/magnet/upload/file"))
            .bearer_auth(&self.api_key)
            .multipart(multipart::Form::new().part("files[]", part))
            .send()
            .await?;
        let data: Value = envelope(response).await?;
        first_upload_id(&data["files"])
    }

    async fn transfer_status(&self, id: &str) -> Result<TransferStatus, DebridError> {
        let data: Value = self.call(Method::POST, "/v4.1/magnet/status", &[("id", id)]).await?;
        // A single magnet may come back as an object or as a one-element array.
        let magnet = match &data["magnets"] {
            Value::Array(items) => items.first().cloned().unwrap_or(Value::Null),
            other => other.clone(),
        };
        if magnet.is_null() {
            return Err(DebridError::LinkDown("magnet not found on AllDebrid".into()));
        }
        let code = magnet["statusCode"].as_i64().unwrap_or(0);
        let size = magnet["size"].as_u64();
        let downloaded = magnet["downloaded"].as_u64().unwrap_or(0);
        let state = match code {
            0 => TransferState::Queued,
            1 => TransferState::Downloading,
            2 | 3 => TransferState::Processing,
            4 => TransferState::Ready,
            _ => TransferState::Error,
        };
        let progress = match (state, size) {
            (TransferState::Ready, _) => 1.0,
            (_, Some(total)) if total > 0 => (downloaded as f64 / total as f64).clamp(0.0, 1.0),
            _ => 0.0,
        };
        Ok(TransferStatus {
            state,
            status_text: magnet["status"].as_str().unwrap_or("Unknown").to_owned(),
            name: magnet["filename"].as_str().map(str::to_owned),
            size,
            progress,
            speed: magnet["downloadSpeed"].as_u64().unwrap_or(0),
            seeders: magnet["seeders"].as_u64().map(|s| s as u32),
        })
    }

    async fn transfer_files(&self, id: &str) -> Result<Vec<RemoteFile>, DebridError> {
        let data: Value = self.call(Method::POST, "/v4/magnet/files", &[("id[]", id)]).await?;
        let magnet = data["magnets"]
            .as_array()
            .and_then(|items| items.iter().find(|m| value_to_string(&m["id"]).as_deref() == Some(id)))
            .cloned()
            .ok_or_else(|| DebridError::Other("magnet files not found".into()))?;
        if !magnet["error"].is_null() {
            return Err(map_error(&magnet["error"]));
        }
        let mut files = Vec::new();
        flatten_files(&magnet["files"], "", &mut files);
        Ok(files)
    }

    async fn delete_transfer(&self, id: &str) -> Result<(), DebridError> {
        let _: Value = self.call(Method::POST, "/v4/magnet/delete", &[("id", id)]).await?;
        Ok(())
    }
}

impl AllDebrid {
    async fn wait_delayed(&self, delayed_id: &str) -> Result<String, DebridError> {
        poll_until(Duration::from_secs(5), Duration::from_secs(15 * 60), || async {
            let data: DelayedData = self.call(Method::POST, "/v4/link/delayed", &[("id", delayed_id)]).await?;
            match data.status {
                2 if !data.link.is_empty() => Ok(Some(data.link)),
                3 => Err(DebridError::LinkDown("AllDebrid could not generate the link".into())),
                _ => Ok(None),
            }
        })
        .await
    }
}

fn best_stream(streams: &[Stream]) -> Option<&Stream> {
    streams.iter().max_by_key(|s| match &s.quality {
        Value::Number(n) => n.as_u64().unwrap_or(0),
        Value::String(q) => q.trim_end_matches('p').parse().unwrap_or(0),
        _ => 0,
    })
}

fn value_to_string(value: &Value) -> Option<String> {
    match value {
        Value::String(s) if !s.is_empty() => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn first_upload_id(items: &Value) -> Result<String, DebridError> {
    let item =
        items.as_array().and_then(|a| a.first()).ok_or_else(|| DebridError::Other("empty upload response".into()))?;
    if !item["error"].is_null() {
        return Err(map_error(&item["error"]));
    }
    value_to_string(&item["id"]).ok_or_else(|| DebridError::Other("upload response without id".into()))
}

/// Walks the `{ n, s, l, e }` tree returned by `/magnet/files`.
fn flatten_files(entries: &Value, prefix: &str, out: &mut Vec<RemoteFile>) {
    for entry in entries.as_array().into_iter().flatten() {
        let name = entry["n"].as_str().unwrap_or("file");
        let path = join_relative(prefix, name);
        if let Some(children) = entry.get("e") {
            flatten_files(children, &path, out);
        } else if let Some(link) = entry["l"].as_str() {
            out.push(RemoteFile { path, size: entry["s"].as_u64(), link: link.to_owned() });
        }
    }
}

pub async fn device_start(http: &reqwest::Client) -> Result<DeviceStart, DebridError> {
    let response = http.get(format!("{API}/v4.1/pin/get")).send().await?;
    let data: Value = envelope(response).await?;
    let user_code = data["pin"].as_str().unwrap_or_default().to_owned();
    Ok(DeviceStart {
        verification_url: data["user_url"].as_str().unwrap_or("https://alldebrid.com/pin/").to_owned(),
        expires_in: data["expires_in"].as_u64().unwrap_or(600),
        state: serde_json::json!({ "pin": user_code, "check": data["check"] }),
        user_code,
    })
}

pub async fn device_poll(http: &reqwest::Client, state: &Value) -> Result<DevicePoll, DebridError> {
    let pin = state["pin"].as_str().unwrap_or_default();
    let check = state["check"].as_str().unwrap_or_default();
    let response = http.post(format!("{API}/v4/pin/check")).form(&[("pin", pin), ("check", check)]).send().await?;
    match envelope::<Value>(response).await {
        Ok(data) if data["activated"] == true => match data["apikey"].as_str() {
            Some(key) => Ok(DevicePoll::Activated(api_key::encode(key))),
            None => Ok(DevicePoll::Pending),
        },
        Ok(_) => Ok(DevicePoll::Pending),
        Err(DebridError::Other(message)) if message.to_ascii_lowercase().contains("expired") => Ok(DevicePoll::Expired),
        Err(error) => Err(error),
    }
}

/// Hosts, streaming sites and redirectors of a `/v4/hosts` answer.
fn host_matcher(data: &Value) -> HostMatcher {
    let mut matcher = HostMatcher::default();
    for group in ["hosts", "streams", "redirectors"] {
        let Some(hosts) = data[group].as_object() else { continue };
        // `status` only says whether AllDebrid's own test of the host passed in the last minutes,
        // and only some hosts are tested (see `HostMatcher`).
        for host in hosts.values() {
            for domain in host["domains"].as_array().into_iter().flatten() {
                matcher.add_domain(domain.as_str().unwrap_or_default());
            }
            // Older responses use `regexps` (array), newer ones `regexp` (string).
            for pattern in host["regexps"].as_array().into_iter().flatten().chain(host.get("regexp")) {
                if let Some(pattern) = pattern.as_str() {
                    matcher.add_pattern(pattern);
                }
            }
        }
    }
    matcher
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_hosts_marked_down() {
        // Extract of /v4/hosts: TurboBit was `false` while AllDebrid unlocked its links fine.
        let data = serde_json::json!({
            "hosts": {
                "turbobit": {
                    "name": "turbobit",
                    "status": false,
                    "domains": ["turbobit.net", "trbt.cc"],
                    "regexp": r"((trbbt|tourbobit|torbobit|tbit|turbobita|trbt)\.(net|cc|com|to)/([a-z0-9]{12}))",
                },
            },
            "redirectors": { "tb": { "status": null, "domains": ["turbobit.net"] } },
        });
        let matcher = host_matcher(&data);
        assert!(matcher.matches("https://torbobit.net/abcdef123456.html?short_domain=trbt.cc"));
        assert!(matcher.matches("https://www.trbt.cc/abcdef123456.html"));
        assert!(!matcher.matches("https://example.com/file.zip"));
    }

    #[test]
    fn flattens_nested_magnet_files() {
        let tree = serde_json::json!([
            { "n": "Show", "e": [
                { "n": "S01E01.mkv", "s": 100, "l": "https://alldebrid.com/f/a" },
                { "n": "Subs", "e": [{ "n": "en.srt", "s": 1, "l": "https://alldebrid.com/f/b" }] }
            ]},
            { "n": "readme.txt", "s": 2, "l": "https://alldebrid.com/f/c" }
        ]);
        let mut files = Vec::new();
        flatten_files(&tree, "", &mut files);
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["Show/S01E01.mkv", "Show/Subs/en.srt", "readme.txt"]);
        assert_eq!(files[0].size, Some(100));
    }

    #[test]
    fn maps_error_codes() {
        let err = |code: &str| map_error(&serde_json::json!({ "code": code, "message": "m" }));
        assert!(matches!(err("AUTH_BAD_APIKEY"), DebridError::Auth(_)));
        assert!(matches!(err("LINK_HOST_NOT_SUPPORTED"), DebridError::Unsupported));
        assert!(matches!(err("LINK_DOWN"), DebridError::LinkDown(_)));
        assert!(matches!(err("LINK_HOST_LIMIT_REACHED"), DebridError::Quota(_)));
        assert!(matches!(err("LINK_TEMPORARY_UNAVAILABLE"), DebridError::Temporary(_)));
    }
}
