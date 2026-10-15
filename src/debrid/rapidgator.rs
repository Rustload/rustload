//! Rapidgator premium accounts — <https://rapidgator.net/article/api/index>
//!
//! API v2 works with a session token obtained from the account's e-mail and password; it is kept
//! with the credentials and renewed when Rapidgator reports the session expired. Accounts with
//! two-factor authentication cannot log in unattended.

use async_trait::async_trait;
use serde_json::Value;

use super::{
    AccountStatus, AuthKind, DebridError, DebridService, HostMatcher, LinkInfo, ProviderInfo, ProviderKind, Unlocked,
    lenient_i64,
    session::{CallError, Session},
    temporary_status,
};
use crate::{
    checksum::{FileHash, HashAlgo},
    util::now,
};

pub const ID: &str = "rapidgator";
const API: &str = "https://rapidgator.net/api/v2";
const DOMAINS: [&str; 3] = ["rapidgator.net", "rapidgator.asia", "rg.to"];

pub const INFO: ProviderInfo = ProviderInfo {
    id: ID,
    name: "Rapidgator",
    website: "https://rapidgator.net",
    api_key_url: "https://rapidgator.net/profile/index",
    device_auth: false,
    magnets: false,
    kind: ProviderKind::Hoster,
    auth: AuthKind::Credentials,
};

pub struct Rapidgator {
    http: reqwest::Client,
    session: Session,
}

impl Rapidgator {
    pub fn from_credentials(raw: &str, http: reqwest::Client) -> Result<Self, DebridError> {
        Ok(Self { http, session: Session::from_credentials(raw)? })
    }

    async fn login(&self) -> Result<String, DebridError> {
        let (login, password) = self.session.login();
        // POST keeps the password out of URLs (and of any error message quoting them).
        let response = self
            .http
            .post(format!("{API}/user/login"))
            .form(&[("login", login.as_str()), ("password", password.as_str())])
            .send()
            .await?;
        let data = envelope(response).await.map_err(|error| match error {
            CallError::Expired => DebridError::Auth("Rapidgator refused the login".into()),
            CallError::Failed(error) => error,
        })?;
        data["token"]
            .as_str()
            .map(str::to_owned)
            .ok_or_else(|| DebridError::Other("Rapidgator returned no session".into()))
    }

    async fn request(&self, path: &str, token: &str, params: &[(&str, &str)]) -> Result<Value, CallError> {
        let mut form = vec![("token", token)];
        form.extend_from_slice(params);
        let response = self.http.post(format!("{API}{path}")).form(&form).send().await?;
        envelope(response).await
    }

    /// Calls an authenticated method, logging in again once if the session expired.
    async fn call(&self, path: &str, params: &[(&str, &str)]) -> Result<Value, DebridError> {
        self.session.call(|| self.login(), |token| async move { self.request(path, &token, params).await }).await
    }
}

/// Unwraps `{ "response": ..., "status": 200, "details": null }`. Some errors use
/// `response_status`/`response_details` instead.
async fn envelope(response: reqwest::Response) -> Result<Value, CallError> {
    let http_status = response.status().as_u16();
    match response.json::<Value>().await {
        Ok(body) => parse_envelope(http_status, &body),
        Err(_) => Err(map_error(http_status, &format!("Rapidgator answered HTTP {http_status}"))),
    }
}

fn parse_envelope(http_status: u16, body: &Value) -> Result<Value, CallError> {
    let status = body.get("status").or_else(|| body.get("response_status")).and_then(Value::as_u64);
    let status = status.map_or(http_status, |s| s as u16);
    if status == 200 && !body["response"].is_null() {
        return Ok(body["response"].clone());
    }
    let details = [&body["details"], &body["response_details"], &body["error"]]
        .into_iter()
        .find_map(Value::as_str)
        .unwrap_or("unknown error");
    Err(map_error(status, details))
}

fn map_error(status: u16, details: &str) -> CallError {
    // "Error: File not found" → "File not found".
    let message = details.trim().trim_start_matches("Error").trim_start_matches([':', '.', ' ']).to_owned();
    let lower = message.to_ascii_lowercase();
    let has = |text: &str| lower.contains(text);
    if has("session") {
        return CallError::Expired;
    }
    CallError::Failed(if has("or password") || has("password cannot be blank") || has("frozen") || has("locked") {
        DebridError::Auth(message)
    } else if has("auth code") || has("2fa") {
        DebridError::Auth(format!("{message} (disable two-factor authentication to use this account)"))
    } else if has("unusual ip") {
        DebridError::Auth(format!("{message} (confirm the new IP address with the link sent by e-mail)"))
    } else if status == 423 || has("exceeded traffic") || has("premium") || has("can download files up to") {
        DebridError::Quota(message)
    } else if status == 404 || has("not found") {
        DebridError::LinkDown(message)
    } else if has("denied by ip") || has("please wait") || temporary_status(status) {
        DebridError::Temporary(message)
    } else if status == 401 {
        return CallError::Expired;
    } else {
        DebridError::Other(message)
    })
}

/// `https://rapidgator.net/file/<32 characters or digits>[/name.html]` → the file id.
fn file_id(link: &str) -> Option<String> {
    let url = url::Url::parse(link).ok()?;
    let mut segments = url.path_segments()?;
    if segments.next()? != "file" {
        return None;
    }
    let id = segments.next()?.to_ascii_lowercase();
    let hash_id = id.len() == 32 && id.chars().all(|c| c.is_ascii_alphanumeric());
    let numeric_id = !id.is_empty() && id.chars().all(|c| c.is_ascii_digit());
    (hash_id || numeric_id).then_some(id)
}

/// Reads the `user` object of `/user/info`; traffic is only meaningful for premium accounts.
fn account_status(user: &Value, now: i64) -> AccountStatus {
    let premium = user["is_premium"] == true;
    AccountStatus {
        username: user["email"].as_str().map(str::to_owned),
        premium,
        premium_until: lenient_i64(&user["premium_end_time"]).filter(|end| premium && *end > now),
        traffic_left: lenient_i64(&user["traffic"]["left"]).filter(|_| premium),
    }
}

fn link_info(file: &Value) -> LinkInfo {
    LinkInfo {
        filename: file["name"].as_str().map(str::to_owned),
        size: lenient_i64(&file["size"]).and_then(|s| u64::try_from(s).ok()),
    }
}

/// `hash` of `/file/info` is the MD5 of the file.
fn file_hash(file: &Value) -> Option<FileHash> {
    file["hash"].as_str().and_then(|hash| FileHash::new(HashAlgo::Md5, hash))
}

#[async_trait]
impl DebridService for Rapidgator {
    fn info(&self) -> ProviderInfo {
        INFO
    }

    async fn account_status(&self) -> Result<AccountStatus, DebridError> {
        let data = self.call("/user/info", &[]).await?;
        let mut status = account_status(&data["user"], now());
        status.username = status.username.or_else(|| Some(self.session.username()));
        Ok(status)
    }

    async fn supported_hosts(&self) -> Result<HostMatcher, DebridError> {
        Ok(HostMatcher::from_domains(&DOMAINS))
    }

    async fn unlock(&self, link: &str, _password: Option<&str>) -> Result<Unlocked, DebridError> {
        let file_id = file_id(link).ok_or(DebridError::Unsupported)?;
        let params = [("file_id", file_id.as_str())];
        let data = match self.call("/file/download", &params).await {
            // This method also answers 404 to accounts that lost premium: only `/file/info` is
            // trusted to tell that a file is gone.
            Err(DebridError::LinkDown(message)) => {
                self.call("/file/info", &params).await?;
                return Err(DebridError::Quota(format!("{message} (is the account still premium?)")));
            }
            result => result?,
        };
        if lenient_i64(&data["delay"]).unwrap_or(0) > 0 {
            return Err(DebridError::Quota("Rapidgator imposes a waiting time: the account is not premium".into()));
        }
        let url = data["download_url"]
            .as_str()
            .ok_or_else(|| DebridError::Other("Rapidgator returned no download link".into()))?
            .to_owned();
        // Name, size and hash are a convenience: the download works without them.
        let (info, hash) = match self.call("/file/info", &params).await {
            Ok(data) => (link_info(&data["file"]), file_hash(&data["file"])),
            Err(error) => {
                tracing::debug!("Rapidgator file info unavailable: {error}");
                (LinkInfo { filename: None, size: None }, None)
            }
        };
        Ok(Unlocked { url, filename: info.filename, size: info.size, hash })
    }

    async fn link_info(&self, link: &str) -> Result<Option<LinkInfo>, DebridError> {
        let Some(file_id) = file_id(link) else { return Ok(None) };
        let data = self.call("/file/info", &[("file_id", &file_id)]).await?;
        Ok(Some(link_info(&data["file"])))
    }

    fn take_updated_credentials(&self) -> Option<String> {
        self.session.take_updated()
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn error(status: u16, details: &str) -> DebridError {
        match map_error(status, details) {
            CallError::Expired => panic!("unexpected session error for {details}"),
            CallError::Failed(error) => error,
        }
    }

    #[test]
    fn extracts_file_ids() {
        let id = "267796c7b59efd11126606dbde8dd1e4";
        assert_eq!(file_id(&format!("https://rapidgator.net/file/{id}/test-setup.exe.html")).as_deref(), Some(id));
        assert_eq!(file_id(&format!("https://rg.to/file/{}", id.to_uppercase())).as_deref(), Some(id));
        assert_eq!(file_id("https://rapidgator.net/file/12345").as_deref(), Some("12345"));
        assert_eq!(file_id("https://rapidgator.net/folder/123/name.html"), None);
        assert_eq!(file_id("https://rapidgator.net/file/short"), None);
    }

    #[test]
    fn claims_every_domain() {
        let matcher = HostMatcher::from_domains(&DOMAINS);
        assert!(matcher.matches("https://rg.to/file/abc"));
        assert!(matcher.matches("https://rapidgator.asia/file/abc"));
        assert!(!matcher.matches("https://1fichier.com/?abc"));
    }

    #[test]
    fn unwraps_documented_responses() {
        let ok = json!({
            "response": { "download_url": "http://pr3.rapidgator.net//?r=download/index&session_id=X", "delay": 0 },
            "status": 200,
            "details": null
        });
        let data = parse_envelope(200, &ok).ok().unwrap();
        assert_eq!(data["delay"], 0);

        let not_found = json!({ "response": null, "status": 404, "details": "Error: File not found" });
        assert!(
            matches!(parse_envelope(404, &not_found), Err(CallError::Failed(DebridError::LinkDown(m))) if m == "File not found")
        );

        let old_style =
            json!({ "response": null, "response_status": 423, "response_details": "Error: Exceeded traffic" });
        assert!(matches!(parse_envelope(200, &old_style), Err(CallError::Failed(DebridError::Quota(_)))));

        let expired = json!({ "response": null, "status": 401, "details": "Error. Session doesn't exist" });
        assert!(matches!(parse_envelope(401, &expired), Err(CallError::Expired)));
    }

    #[test]
    fn maps_errors() {
        assert!(matches!(error(401, "Error: Login or password is wrong"), DebridError::Auth(_)));
        assert!(matches!(error(401, "Error: Invalid auth code"), DebridError::Auth(_)));
        assert!(matches!(error(403, "Error: User is FROZEN"), DebridError::Auth(_)));
        assert!(matches!(error(403, "Error: This file can be downloaded by premium only"), DebridError::Quota(_)));
        assert!(matches!(error(503, "Download temporarily unavailable"), DebridError::Temporary(_)));
        assert!(matches!(error(403, "Denied by IP"), DebridError::Temporary(_)));
        assert!(matches!(map_error(401, "Error: Something else"), CallError::Expired));
        assert!(matches!(map_error(404, "Error: Session not found"), CallError::Expired));
    }

    #[test]
    fn reads_user_info() {
        let now = 1_700_000_000;
        let premium = json!({
            "email": "john@example.com",
            "is_premium": true,
            "premium_end_time": 1_800_000_000,
            "state": 1,
            "traffic": { "total": "1099511627776", "left": 549_755_813_888u64 },
            "storage": { "total": "4398046511104", "left": 4_398_035_213_138u64 }
        });
        let status = account_status(&premium, now);
        assert!(status.premium);
        assert_eq!(status.username.as_deref(), Some("john@example.com"));
        assert_eq!(status.premium_until, Some(1_800_000_000));
        assert_eq!(status.traffic_left, Some(549_755_813_888));

        let free = json!({ "email": "john@example.com", "is_premium": false, "premium_end_time": null,
                           "traffic": { "total": null, "left": null } });
        let status = account_status(&free, now);
        assert!(!status.premium);
        assert_eq!((status.premium_until, status.traffic_left), (None, None));
    }

    #[test]
    fn reads_file_info() {
        let file = json!({
            "file_id": "267796c7b59efd11126606dbde8dd1e4",
            "name": "test-setup.exe",
            "hash": "097aa1113bf9c60994c2425c7547b760",
            "size": 597304
        });
        let info = link_info(&file);
        assert_eq!(info.filename.as_deref(), Some("test-setup.exe"));
        assert_eq!(info.size, Some(597_304));
        assert_eq!(file_hash(&file).unwrap().to_string(), "md5:097aa1113bf9c60994c2425c7547b760");
        assert_eq!(file_hash(&json!({ "hash": "" })), None);
    }
}
