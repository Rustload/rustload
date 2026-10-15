//! Premiumize.me — <https://www.premiumize.me/api>

use std::time::Duration;

use async_trait::async_trait;
use reqwest::{RequestBuilder, multipart};
use serde::de::DeserializeOwned;
use serde_json::Value;

use super::{
    AccountStatus, AuthKind, DebridError, DebridService, HostMatcher, ProviderInfo, ProviderKind, RemoteFile,
    TransferState, TransferStatus, Unlocked, api_key, http_failure, mark_direct, poll_until, strip_direct,
};
use crate::util::join_relative;

pub const ID: &str = "premiumize";
const API: &str = "https://www.premiumize.me/api";

pub const INFO: ProviderInfo = ProviderInfo {
    id: ID,
    name: "Premiumize.me",
    website: "https://www.premiumize.me",
    api_key_url: "https://www.premiumize.me/account",
    device_auth: false,
    magnets: true,
    kind: ProviderKind::Debrid,
    auth: AuthKind::ApiKey,
};

pub struct Premiumize {
    http: reqwest::Client,
    api_key: String,
}

impl Premiumize {
    pub fn from_credentials(raw: &str, http: reqwest::Client) -> Result<Self, DebridError> {
        Ok(Self { http, api_key: api_key::decode(raw)? })
    }

    /// Premiumize answers HTTP 200 with `{ "status": "error", "code": ... }` on failure.
    async fn send<T: DeserializeOwned>(&self, request: RequestBuilder) -> Result<T, DebridError> {
        let response = request.bearer_auth(&self.api_key).send().await?;
        if let Some(error) = http_failure("Premiumize", response.status()) {
            return Err(error);
        }
        let body: Value = response
            .json()
            .await
            .map_err(|e| DebridError::Temporary(format!("invalid response: {}", e.without_url())))?;
        if body["status"] == "error" {
            return Err(map_error(&body));
        }
        serde_json::from_value(body).map_err(|e| DebridError::Other(format!("unexpected Premiumize response: {e}")))
    }

    async fn get<T: DeserializeOwned>(&self, path: &str, query: &[(&str, &str)]) -> Result<T, DebridError> {
        self.send(self.http.get(format!("{API}{path}")).query(query)).await
    }

    async fn post<T: DeserializeOwned>(&self, path: &str, form: &[(&str, &str)]) -> Result<T, DebridError> {
        self.send(self.http.post(format!("{API}{path}")).form(form)).await
    }

    async fn transfer(&self, id: &str) -> Result<Value, DebridError> {
        let list: Value = self.get("/transfer/list", &[]).await?;
        list["transfers"]
            .as_array()
            .and_then(|items| items.iter().find(|t| t["id"].as_str() == Some(id)))
            .cloned()
            .ok_or_else(|| DebridError::LinkDown("transfer not found on Premiumize".into()))
    }

    /// Lists the files of a finished transfer (single file or folder tree).
    async fn finished_files(&self, transfer: &Value) -> Result<Vec<RemoteFile>, DebridError> {
        if let Some(file_id) = transfer["file_id"].as_str() {
            let item: Value = self.get("/item/details", &[("id", file_id)]).await?;
            return Ok(vec![RemoteFile {
                path: item["name"].as_str().unwrap_or("file").to_owned(),
                size: item["size"].as_u64(),
                link: mark_direct(item["link"].as_str().unwrap_or_default()),
            }]);
        }
        let folder_id =
            transfer["folder_id"].as_str().ok_or_else(|| DebridError::Other("transfer has no files".into()))?;
        let mut files = Vec::new();
        let mut pending = vec![(folder_id.to_owned(), String::new())];
        while let Some((id, prefix)) = pending.pop() {
            let folder: Value = self.get("/folder/list", &[("id", &id)]).await?;
            for entry in folder["content"].as_array().into_iter().flatten() {
                let name = entry["name"].as_str().unwrap_or("file");
                let path = join_relative(&prefix, name);
                match (entry["type"].as_str(), entry["id"].as_str(), entry["link"].as_str()) {
                    (Some("folder"), Some(child), _) => pending.push((child.to_owned(), path)),
                    (_, _, Some(link)) => {
                        files.push(RemoteFile { path, size: entry["size"].as_u64(), link: mark_direct(link) })
                    }
                    _ => {}
                }
            }
        }
        Ok(files)
    }
}

fn map_error(body: &Value) -> DebridError {
    let message = body["message"].as_str().unwrap_or("unknown error").to_owned();
    match body["code"].as_str().unwrap_or_default() {
        "authentication_failed" | "permission_denied" => DebridError::Auth(message),
        "service_unsupported" => DebridError::Unsupported,
        "not_found" => DebridError::LinkDown(message),
        "service_limit_reached" | "account_limit_reached" => DebridError::Quota(message),
        "link_generation_failed"
        | "transient_error"
        | "service_down"
        | "rate_limit_reached"
        | "semi_permanent_error" => DebridError::Temporary(message),
        _ if message.to_ascii_lowercase().contains("not logged in") => DebridError::Auth(message),
        _ => DebridError::Other(message),
    }
}

#[async_trait]
impl DebridService for Premiumize {
    fn info(&self) -> ProviderInfo {
        INFO
    }

    async fn account_status(&self) -> Result<AccountStatus, DebridError> {
        let data: Value = self.get("/account/info", &[]).await?;
        let premium_until = data["premium_until"].as_i64().filter(|t| *t > crate::util::now());
        Ok(AccountStatus {
            username: data["customer_id"]
                .as_str()
                .map(str::to_owned)
                .or_else(|| data["customer_id"].as_u64().map(|n| n.to_string())),
            premium: premium_until.is_some(),
            premium_until,
            traffic_left: None,
        })
    }

    async fn supported_hosts(&self) -> Result<HostMatcher, DebridError> {
        let data: Value = self.get("/services/list", &[]).await?;
        let mut matcher = HostMatcher::default();
        for list in ["directdl", "queue"] {
            for host in data[list].as_array().into_iter().flatten().filter_map(Value::as_str) {
                matcher.add_domain(host);
                for alias in data["aliases"][host].as_array().into_iter().flatten().filter_map(Value::as_str) {
                    matcher.add_domain(alias);
                }
                for pattern in data["regexpatterns"][host].as_array().into_iter().flatten().filter_map(Value::as_str) {
                    matcher.add_pattern(pattern);
                }
            }
        }
        Ok(matcher)
    }

    async fn unlock(&self, link: &str, password: Option<&str>) -> Result<Unlocked, DebridError> {
        if let Some(url) = strip_direct(link) {
            return Ok(Unlocked { url: url.to_owned(), filename: None, size: None, hash: None });
        }
        if password.is_none() {
            match self.post::<Value>("/transfer/directdl", &[("src", link)]).await {
                Ok(data) => {
                    let entry = &data["content"][0];
                    if let Some(url) = entry["link"].as_str() {
                        return Ok(Unlocked {
                            url: url.to_owned(),
                            filename: entry["path"].as_str().map(|p| p.rsplit('/').next().unwrap_or(p).to_owned()),
                            size: entry["size"].as_u64(),
                            hash: None,
                        });
                    }
                }
                // Hosts in the "queue" list only work through cloud transfers, handled below.
                Err(DebridError::Unsupported | DebridError::Other(_)) => {}
                Err(error) => return Err(error),
            }
        }

        // Slow path: download to the Premiumize cloud first, then fetch the file link.
        let mut form = vec![("src", link)];
        if let Some(password) = password {
            form.push(("password", password));
        }
        let created: Value = self.post("/transfer/create", &form).await?;
        let id = created["id"].as_str().ok_or_else(|| DebridError::Other("no transfer id returned".into()))?.to_owned();
        let transfer = poll_until(Duration::from_secs(10), Duration::from_secs(30 * 60), || async {
            let transfer = self.transfer(&id).await?;
            match transfer["status"].as_str() {
                Some("finished" | "seeding") => Ok(Some(transfer)),
                Some("queued" | "waiting" | "running") => Ok(None),
                _ => Err(DebridError::LinkDown(transfer["message"].as_str().unwrap_or("transfer failed").to_owned())),
            }
        })
        .await?;
        let file = self
            .finished_files(&transfer)
            .await?
            .into_iter()
            .next()
            .ok_or_else(|| DebridError::Other("no file".into()))?;
        Ok(Unlocked {
            url: strip_direct(&file.link).unwrap_or(&file.link).to_owned(),
            filename: Some(file.path),
            size: file.size,
            hash: None,
        })
    }

    async fn add_magnet(&self, magnet: &str) -> Result<String, DebridError> {
        let created: Value = self.post("/transfer/create", &[("src", magnet)]).await?;
        created["id"].as_str().map(str::to_owned).ok_or_else(|| DebridError::Other("no transfer id returned".into()))
    }

    async fn add_torrent(&self, file_name: &str, data: Vec<u8>) -> Result<String, DebridError> {
        let part = multipart::Part::bytes(data).file_name(file_name.to_owned());
        let request =
            self.http.post(format!("{API}/transfer/create")).multipart(multipart::Form::new().part("src", part));
        let created: Value = self.send(request).await?;
        created["id"].as_str().map(str::to_owned).ok_or_else(|| DebridError::Other("no transfer id returned".into()))
    }

    async fn transfer_status(&self, id: &str) -> Result<TransferStatus, DebridError> {
        let transfer = self.transfer(id).await?;
        let status = transfer["status"].as_str().unwrap_or("unknown");
        let state = match status {
            "queued" | "waiting" => TransferState::Queued,
            "running" => TransferState::Downloading,
            "finished" | "seeding" => TransferState::Ready,
            _ => TransferState::Error,
        };
        let message = transfer["message"].as_str().filter(|m| !m.is_empty()).unwrap_or(status);
        Ok(TransferStatus {
            state,
            status_text: message.to_owned(),
            name: transfer["name"].as_str().map(str::to_owned),
            size: None,
            progress: transfer["progress"].as_f64().unwrap_or(0.0).clamp(0.0, 1.0),
            speed: 0,
            seeders: None,
        })
    }

    async fn transfer_files(&self, id: &str) -> Result<Vec<RemoteFile>, DebridError> {
        let transfer = self.transfer(id).await?;
        self.finished_files(&transfer).await
    }

    async fn delete_transfer(&self, id: &str) -> Result<(), DebridError> {
        // Deleting the transfer keeps its files in the cloud: remove them too.
        if let Ok(transfer) = self.transfer(id).await {
            if let Some(file_id) = transfer["file_id"].as_str() {
                let _: Result<Value, _> = self.post("/item/delete", &[("id", file_id)]).await;
            } else if let Some(folder_id) = transfer["folder_id"].as_str() {
                let _: Result<Value, _> = self.post("/folder/delete", &[("id", folder_id)]).await;
            }
        }
        let _: Value = self.post("/transfer/delete", &[("id", id)]).await?;
        Ok(())
    }
}
