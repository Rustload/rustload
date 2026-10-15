//! Keep2Share, FileBoom and Tezfiles premium accounts — <https://keep2share.github.io/api/>
//!
//! The three sites share one API, each on its own domain, with separate accounts. Accounts log in
//! with their e-mail and password; the session token is kept with the credentials and renewed when
//! it expires. When a site asks for a captcha at login, the user has to log in once on the website
//! from the same network.

use async_trait::async_trait;
use chrono::DateTime;
use serde_json::{Value, json};

use super::{
    AccountStatus, AuthKind, DebridError, DebridService, HostMatcher, LinkInfo, ProviderInfo, ProviderKind, Unlocked,
    http_failure, lenient_i64,
    session::{CallError, Session},
};
use crate::{
    checksum::{FileHash, HashAlgo},
    util::now,
};

/// One of the sites using the Keep2Share API.
pub struct Project {
    pub info: ProviderInfo,
    api: &'static str,
    domains: &'static [&'static str],
}

const fn define(
    id: &'static str,
    name: &'static str,
    website: &'static str,
    api: &'static str,
    domains: &'static [&'static str],
) -> Project {
    Project {
        info: ProviderInfo {
            id,
            name,
            website,
            api_key_url: website,
            device_auth: false,
            magnets: false,
            kind: ProviderKind::Hoster,
            auth: AuthKind::Credentials,
        },
        api,
        domains,
    }
}

pub const PROJECTS: [Project; 3] = [
    define(
        "keep2share",
        "Keep2Share",
        "https://k2s.cc",
        "https://keep2share.cc/api/v2",
        &["k2s.cc", "keep2share.cc", "k2share.cc", "keep2s.cc", "keep2.cc"],
    ),
    define("fileboom", "FileBoom", "https://fboom.me", "https://fboom.me/api/v2", &["fboom.me", "fileboom.me"]),
    define(
        "tezfiles",
        "Tezfiles",
        "https://tezfiles.com",
        "https://tezfiles.com/api/v2",
        &["tezfiles.com", "publish2.me"],
    ),
];

pub fn project(id: &str) -> Option<&'static Project> {
    PROJECTS.iter().find(|project| project.info.id == id)
}

pub struct Keep2Share {
    project: &'static Project,
    http: reqwest::Client,
    session: Session,
}

impl Keep2Share {
    pub fn from_credentials(project: &'static Project, raw: &str, http: reqwest::Client) -> Result<Self, DebridError> {
        Ok(Self { project, http, session: Session::from_credentials(raw)? })
    }

    /// POSTs a JSON request; `{ "status": "success", ... }` is returned as is.
    async fn request(&self, method: &str, mut body: Value, token: Option<&str>) -> Result<Value, CallError> {
        if let Some(token) = token {
            body["auth_token"] = json!(token);
        }
        let response = self.http.post(format!("{}/{method}", self.project.api)).json(&body).send().await?;
        let status = response.status().as_u16();
        let name = self.project.info.name;
        match response.json::<Value>().await {
            Ok(body) => parse_response(body),
            Err(_) => Err(CallError::Failed(
                http_failure(name, status)
                    .unwrap_or_else(|| DebridError::Other(format!("unexpected {name} response (HTTP {status})"))),
            )),
        }
    }

    async fn login(&self) -> Result<String, DebridError> {
        let (username, password) = self.session.login();
        let data = self.request("login", json!({ "username": username, "password": password }), None).await?;
        data["auth_token"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| DebridError::Other(format!("{} returned no session", self.project.info.name)))
    }

    /// Calls an authenticated method, logging in again once if the session expired.
    async fn call(&self, method: &str, body: Value) -> Result<Value, DebridError> {
        let body = &body;
        let request = |token: String| async move { self.request(method, body.clone(), Some(&token)).await };
        self.session.call(|| self.login(), request).await
    }

    async fn file_info(&self, file_id: &str) -> Result<Value, DebridError> {
        let data = self.call("getFilesInfo", json!({ "ids": [file_id] })).await?;
        let file = data["files"].get(0).cloned().unwrap_or(Value::Null);
        // Removed and unknown files come back with `is_available` false or null.
        if file["is_available"] != true {
            return Err(DebridError::LinkDown("file not found or removed".into()));
        }
        Ok(file)
    }
}

/// Errors are `{ "status": "error", "code": <HTTP>, "errorCode": <n>, "message": "...",
/// "errors": [{ "code": <detail>, "message": "..." }] }`.
fn parse_response(body: Value) -> Result<Value, CallError> {
    if body["status"] == "success" {
        return Ok(body);
    }
    let code = body["code"].as_u64().unwrap_or(0);
    let error_code = body["errorCode"].as_u64().unwrap_or(0);
    let detail = &body["errors"][0];
    let message = detail["message"].as_str().or(body["message"].as_str()).unwrap_or("unknown error").to_owned();
    Err(map_error(code, error_code, detail["code"].as_u64(), message))
}

fn map_error(code: u64, error_code: u64, detail: Option<u64>, message: String) -> CallError {
    CallError::Failed(match error_code {
        // Not authorized, or the token belongs to another IP address: log in again.
        10 | 75 => return CallError::Expired,
        33 => DebridError::Auth(format!(
            "{message} (a captcha is required: log in once on the website from this network, then check the account again)"
        )),
        70 | 72 | 74 | 76 => DebridError::Auth(message),
        71 => DebridError::Temporary(message),
        73 | 77 => DebridError::Other(format!("{message} (this network is refused)")),
        20 | 22 => DebridError::LinkDown(message),
        // File not available: private or reserved to the uploader's store subscribers.
        21 | 42 if matches!(detail, Some(8 | 9)) => DebridError::LinkDown(message),
        // Download count, traffic or size limits, premium-only files, captcha or waiting time: the
        // account is not premium (anymore) or used up its quota.
        1 | 2 | 3 | 21 | 30 | 31 | 40 | 41 | 42 => DebridError::Quota(message),
        23 => DebridError::Unsupported,
        _ if code == 429 || code >= 500 => DebridError::Temporary(message),
        _ if code == 401 => return CallError::Expired,
        _ => DebridError::Other(message),
    })
}

/// `https://k2s.cc/file/<id>[/name]` (also `/f/`, `/preview/`, `/file/info/`) → the file id.
fn file_id(link: &str) -> Option<String> {
    let url = url::Url::parse(link).ok()?;
    let mut segments = url.path_segments()?.filter(|s| !s.is_empty());
    if !matches!(segments.next()?, "f" | "file" | "preview") {
        return None;
    }
    let mut id = segments.next()?;
    if id == "info" {
        id = segments.next()?;
    }
    let valid = id.len() >= 13 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    valid.then(|| id.to_ascii_lowercase())
}

/// `account_expires` is documented as a timestamp but sent as an ISO date.
fn expiry(value: &Value) -> Option<i64> {
    match value {
        Value::String(text) => DateTime::parse_from_rfc3339(text).ok().map(|date| date.timestamp()),
        other => lenient_i64(other),
    }
    .filter(|t| *t > 0)
}

fn account_status(data: &Value, username: String, now: i64) -> AccountStatus {
    let premium_until = expiry(&data["account_expires"]).filter(|end| *end > now);
    AccountStatus {
        username: Some(username),
        premium: premium_until.is_some(),
        premium_until,
        traffic_left: lenient_i64(&data["available_traffic"]).filter(|_| premium_until.is_some()),
    }
}

fn link_info(file: &Value) -> LinkInfo {
    LinkInfo {
        filename: file["name"].as_str().map(str::to_owned),
        size: lenient_i64(&file["size"]).and_then(|s| u64::try_from(s).ok()),
    }
}

/// The MD5 is only published to the owner of the file.
fn file_hash(file: &Value) -> Option<FileHash> {
    file["md5"].as_str().and_then(|hash| FileHash::new(HashAlgo::Md5, hash))
}

#[async_trait]
impl DebridService for Keep2Share {
    fn info(&self) -> ProviderInfo {
        self.project.info.clone()
    }

    async fn account_status(&self) -> Result<AccountStatus, DebridError> {
        let data = self.call("accountInfo", json!({})).await?;
        Ok(account_status(&data, self.session.username(), now()))
    }

    async fn supported_hosts(&self) -> Result<HostMatcher, DebridError> {
        Ok(HostMatcher::from_domains(self.project.domains))
    }

    async fn unlock(&self, link: &str, _password: Option<&str>) -> Result<Unlocked, DebridError> {
        let file_id = file_id(link).ok_or(DebridError::Unsupported)?;
        let data = self.call("getUrl", json!({ "file_id": file_id })).await?;
        let url = data["url"]
            .as_str()
            .ok_or_else(|| DebridError::Other(format!("{} returned no download link", self.project.info.name)))?
            .to_owned();
        // Name, size and hash are a convenience: the download works without them.
        let (info, hash) = match self.file_info(&file_id).await {
            Ok(file) => (link_info(&file), file_hash(&file)),
            Err(error) => {
                tracing::debug!("{} file info unavailable: {error}", self.project.info.name);
                (LinkInfo { filename: None, size: None }, None)
            }
        };
        Ok(Unlocked { url, filename: info.filename, size: info.size, hash })
    }

    async fn link_info(&self, link: &str) -> Result<Option<LinkInfo>, DebridError> {
        let Some(file_id) = file_id(link) else { return Ok(None) };
        Ok(Some(link_info(&self.file_info(&file_id).await?)))
    }

    fn take_updated_credentials(&self) -> Option<String> {
        self.session.take_updated()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn error(body: Value) -> DebridError {
        match parse_response(body) {
            Err(CallError::Failed(error)) => error,
            Err(CallError::Expired) => panic!("unexpected session error"),
            Ok(_) => panic!("unexpected success"),
        }
    }

    #[test]
    fn extracts_file_ids() {
        assert_eq!(file_id("https://k2s.cc/file/96a9d7445484c/video.mp4").as_deref(), Some("96a9d7445484c"));
        assert_eq!(file_id("https://keep2share.cc/file/info/96a9d7445484c").as_deref(), Some("96a9d7445484c"));
        assert_eq!(file_id("https://fboom.me/f/96A9D7445484C").as_deref(), Some("96a9d7445484c"));
        assert_eq!(file_id("https://tezfiles.com/folder/96a9d7445484c"), None);
        assert_eq!(file_id("https://k2s.cc/file/short"), None);
    }

    #[test]
    fn each_project_claims_its_domains() {
        let k2s = HostMatcher::from_domains(project("keep2share").unwrap().domains);
        assert!(k2s.matches("https://k2s.cc/file/96a9d7445484c"));
        assert!(k2s.matches("https://keep2s.cc/file/96a9d7445484c"));
        assert!(!k2s.matches("https://fboom.me/file/96a9d7445484c"));
        let fileboom = HostMatcher::from_domains(project("fileboom").unwrap().domains);
        assert!(fileboom.matches("https://fileboom.me/file/96a9d7445484c"));
        let tezfiles = HostMatcher::from_domains(project("tezfiles").unwrap().domains);
        assert!(tezfiles.matches("https://publish2.me/file/96a9d7445484c"));
        assert!(project("rapidgator").is_none());
    }

    #[test]
    fn maps_documented_errors() {
        let captcha = json!({ "message": "Please verify your request via re captcha challenge", "status": "error",
                              "code": 406, "errorCode": 33 });
        assert!(matches!(error(captcha), DebridError::Auth(_)));
        let banned = json!({ "status": "error", "code": 403, "errorCode": 72, "message": "Account banned" });
        assert!(matches!(error(banned), DebridError::Auth(_)));
        let not_found = json!({ "status": "error", "code": 406, "errorCode": 20, "message": "File not found" });
        assert!(matches!(error(not_found), DebridError::LinkDown(_)));
        let traffic = json!({ "status": "error", "code": 406, "errorCode": 21, "message": "File is not available",
                              "errors": [{ "code": 2, "message": "Traffic limit exceed" }] });
        assert!(matches!(error(traffic), DebridError::Quota(m) if m == "Traffic limit exceed"));
        let private = json!({ "status": "error", "code": 406, "errorCode": 21, "message": "File is not available",
                              "errors": [{ "code": 8 }] });
        assert!(matches!(error(private), DebridError::LinkDown(_)));
        let captcha_download =
            json!({ "status": "error", "code": 406, "errorCode": 30, "message": "Captcha required" });
        assert!(matches!(error(captcha_download), DebridError::Quota(_)));
        let unauthorized = json!({ "message": "You are not authorized for this action", "status": "error",
                                   "code": 403, "errorCode": 10 });
        assert!(matches!(parse_response(unauthorized), Err(CallError::Expired)));
    }

    #[test]
    fn reads_account_info() {
        let now = 1_600_000_000;
        let data = json!({ "status": "success", "code": 200, "available_traffic": 53_687_091_200u64,
                           "account_expires": "2022-04-24T00:00:00.000Z" });
        let status = account_status(&data, "me@example.com".into(), now);
        assert!(status.premium);
        assert_eq!(status.premium_until, Some(1_650_758_400));
        assert_eq!(status.traffic_left, Some(53_687_091_200));
        assert_eq!(status.username.as_deref(), Some("me@example.com"));

        let numeric = json!({ "available_traffic": 0, "account_expires": 1_650_758_400 });
        assert_eq!(account_status(&numeric, String::new(), now).premium_until, Some(1_650_758_400));
        let free = json!({ "available_traffic": 0, "account_expires": null });
        let status = account_status(&free, String::new(), now);
        assert!(!status.premium);
        assert_eq!(status.traffic_left, None);
    }

    #[test]
    fn reads_file_info() {
        let body = json!({ "code": 200, "status": "success", "files": [{ "access": "public", "id": "da4a6352cb666",
            "is_available": true, "is_folder": false, "md5": "6661db843941f4b406e590c21caa9ca1",
            "name": "Super video.mp4", "size": 479_125_093 }] });
        let data = parse_response(body).ok().unwrap();
        let info = link_info(&data["files"][0]);
        assert_eq!(info.filename.as_deref(), Some("Super video.mp4"));
        assert_eq!(info.size, Some(479_125_093));
        assert_eq!(file_hash(&data["files"][0]).unwrap().to_string(), "md5:6661db843941f4b406e590c21caa9ca1");
        assert_eq!(file_hash(&json!({ "md5": null })), None);
    }
}
