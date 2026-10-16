//! Debrid-Link — <https://debrid-link.com/api_doc/v2>

use std::time::Duration;

use async_trait::async_trait;
use reqwest::{RequestBuilder, multipart};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{
    AccountStatus, AuthKind, DebridError, DebridService, HostMatcher, ProviderInfo, ProviderKind, RemoteFile,
    TransferState, TransferStatus, Unlocked, api_key, http_failure, mark_direct, poll_until, strip_direct,
};
use crate::util::now;

pub const ID: &str = "debridlink";
const API: &str = "https://debrid-link.com/api/v2";

pub const INFO: ProviderInfo = ProviderInfo {
    id: ID,
    name: "Debrid-Link",
    website: "https://debrid-link.com",
    api_key_url: "https://debrid-link.com/webapp/apikey",
    device_auth: false,
    magnets: true,
    kind: ProviderKind::Debrid,
    auth: AuthKind::ApiKey,
};

pub struct DebridLink {
    http: reqwest::Client,
    api_key: String,
}

impl DebridLink {
    pub fn from_credentials(raw: &str, http: reqwest::Client) -> Result<Self, DebridError> {
        Ok(Self { http, api_key: api_key::decode(raw)? })
    }

    /// Unwraps `{ "success": true, "value": ... }`; errors come with an HTTP error status.
    async fn send<T: DeserializeOwned>(&self, request: RequestBuilder) -> Result<T, DebridError> {
        let response = request.bearer_auth(&self.api_key).send().await?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if body["success"] == true {
            return serde_json::from_value(body["value"].clone())
                .map_err(|e| DebridError::Other(format!("unexpected Debrid-Link response: {e}")));
        }
        match http_failure("Debrid-Link", status) {
            Some(error) if body.is_null() => Err(error),
            _ => Err(map_error(&body)),
        }
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T, DebridError> {
        self.send(self.http.get(format!("{API}{path}")).query(query)).await
    }

    async fn torrent(&self, id: &str) -> Result<Value, DebridError> {
        let list: Vec<Value> = self.get("/seedbox/list", &[("ids", id), ("structureType", "list")]).await?;
        list.into_iter().next().ok_or_else(|| DebridError::LinkDown("torrent not found on Debrid-Link".into()))
    }
}

fn map_error(body: &Value) -> DebridError {
    let code = body["error"].as_str().unwrap_or("unknown");
    let message = body["error_description"].as_str().map(str::to_owned).unwrap_or_else(|| code.to_owned());
    match code {
        "badToken" | "invalid_grant" | "invalid_client" | "access_denied" | "unverifiedEmail" => {
            DebridError::Auth(message)
        }
        "hostNotValid" => DebridError::Unsupported,
        "fileNotFound" | "badFileUrl" | "badFilePassword" | "torrentTooBig" | "badTorrentFile" | "badId"
        | "infringingFile" => DebridError::LinkDown(message),
        "maxLink" | "maxLinkHost" | "maxData" | "maxDataHost" | "maxTorrent" | "notFreeHost" => {
            DebridError::Quota(message)
        }
        "floodDetected" | "internalError" | "server_error" | "freeServerOverload" | "notDebrid" | "maintenanceHost"
        | "fileNotAvailable" | "notAddTorrent" | "maxTransfer" => DebridError::Temporary(message),
        _ => DebridError::Other(message),
    }
}

#[async_trait]
impl DebridService for DebridLink {
    fn info(&self) -> ProviderInfo {
        INFO
    }

    async fn account_status(&self) -> Result<AccountStatus, DebridError> {
        let data: Value = self.get("/account/infos", &[]).await?;
        let kind = data["accountType"].as_i64().unwrap_or(0);
        let seconds_left = data["premiumLeft"].as_i64().unwrap_or(0);
        Ok(AccountStatus {
            username: data["username"].as_str().map(str::to_owned),
            premium: kind >= 1,
            // Lifetime accounts (type 2) never expire.
            premium_until: (kind == 1 && seconds_left > 0).then(|| now() + seconds_left),
            traffic_left: None,
        })
    }

    async fn supported_hosts(&self) -> Result<HostMatcher, DebridError> {
        let hosts: Vec<Value> = self.get("/downloader/hosts", &[("keys", "name,type,status,domains,regexs")]).await?;
        let mut matcher = HostMatcher::default();
        // Hosts are kept whatever their `status`: the unlock says whether one works today (see
        // `HostMatcher`).
        for host in &hosts {
            for domain in host["domains"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                matcher.add_domain(domain);
            }
            for pattern in host["regexs"].as_array().into_iter().flatten().filter_map(Value::as_str) {
                matcher.add_pattern(pattern);
            }
        }
        Ok(matcher)
    }

    async fn unlock(&self, link: &str, password: Option<&str>) -> Result<Unlocked, DebridError> {
        if let Some(url) = strip_direct(link) {
            return Ok(Unlocked { url: url.to_owned(), filename: None, size: None, hash: None });
        }
        let mut form = vec![("url", link)];
        if let Some(password) = password {
            form.push(("password", password));
        }
        let value: Value = self.send(self.http.post(format!("{API}/downloader/add")).form(&form)).await?;
        // Folder links return an array: take the first file.
        let mut item = match value {
            Value::Array(items) => items.into_iter().next().unwrap_or(Value::Null),
            other => other,
        };
        if item["isProcessing"] == true {
            let id = item["id"].as_str().unwrap_or_default().to_owned();
            item = poll_until(Duration::from_secs(5), Duration::from_secs(15 * 60), || async {
                let list: Vec<Value> = self.get("/downloader/list", &[("ids", &id)]).await?;
                Ok(list.into_iter().next().filter(|i| i["isProcessing"] != true))
            })
            .await?;
        }
        let url = item["downloadUrl"].as_str().ok_or_else(|| DebridError::Other("no download link returned".into()))?;
        Ok(Unlocked {
            url: url.to_owned(),
            filename: item["name"].as_str().map(str::to_owned),
            size: item["size"].as_u64(),
            hash: None,
        })
    }

    async fn add_magnet(&self, magnet: &str) -> Result<String, DebridError> {
        let value: Value = self.send(self.http.post(format!("{API}/seedbox/add")).form(&[("url", magnet)])).await?;
        value["id"].as_str().map(str::to_owned).ok_or_else(|| DebridError::Other("no torrent id returned".into()))
    }

    async fn add_torrent(&self, file_name: &str, data: Vec<u8>) -> Result<String, DebridError> {
        let part = multipart::Part::bytes(data).file_name(file_name.to_owned());
        let request = self.http.post(format!("{API}/seedbox/add")).multipart(multipart::Form::new().part("file", part));
        let value: Value = self.send(request).await?;
        value["id"].as_str().map(str::to_owned).ok_or_else(|| DebridError::Other("no torrent id returned".into()))
    }

    async fn transfer_status(&self, id: &str) -> Result<TransferStatus, DebridError> {
        let torrent = self.torrent(id).await?;
        let files_ready = torrent["files"].as_array().is_some_and(|files| {
            !files.is_empty() && files.iter().all(|f| f["downloadPercent"].as_f64() == Some(100.0))
        });
        let (state, text) = match torrent["status"].as_i64().unwrap_or(-1) {
            8 | 100 if files_ready => (TransferState::Ready, "Ready"),
            8 | 100 | 2 => (TransferState::Processing, "Processing"),
            4 => (TransferState::Downloading, "Downloading"),
            0 | 1 => (TransferState::Queued, "Queued"),
            _ => (TransferState::Error, "Error"),
        };
        Ok(TransferStatus {
            state,
            status_text: if torrent["srvMaint"] == true { "Server maintenance".into() } else { text.into() },
            name: torrent["name"].as_str().map(str::to_owned),
            size: torrent["totalSize"].as_u64(),
            progress: torrent["downloadPercent"].as_f64().unwrap_or(0.0) / 100.0,
            speed: torrent["downloadSpeed"].as_u64().unwrap_or(0),
            seeders: torrent["peersConnected"].as_u64().map(|p| p as u32),
        })
    }

    async fn transfer_files(&self, id: &str) -> Result<Vec<RemoteFile>, DebridError> {
        let torrent = self.torrent(id).await?;
        Ok(torrent["files"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|file| {
                Some(RemoteFile {
                    path: file["name"].as_str()?.to_owned(),
                    size: file["size"].as_u64(),
                    link: mark_direct(file["downloadUrl"].as_str()?),
                })
            })
            .collect())
    }

    async fn delete_transfer(&self, id: &str) -> Result<(), DebridError> {
        let _: Value = self.send(self.http.delete(format!("{API}/seedbox/{id}/remove"))).await?;
        Ok(())
    }
}
