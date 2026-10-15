//! 1fichier premium accounts — <https://1fichier.com/api.html>
//!
//! The API takes the account's API key (premium accounts only) and accepts 3 requests per second;
//! repeated errors get the IP address banned for a while. Calls are therefore spaced, and links
//! are not checked through the API before they start.

use std::time::Duration;

use async_trait::async_trait;
use chrono::{Datelike, Days, NaiveDate, NaiveDateTime};
use serde_json::{Value, json};
use tokio::{sync::Mutex, time::Instant};

use super::{
    AccountStatus, AuthKind, DebridError, DebridService, HostMatcher, ProviderInfo, ProviderKind, Unlocked, api_key,
    lenient_i64, temporary_status,
};
use crate::util::now;

pub const ID: &str = "1fichier";
const API: &str = "https://api.1fichier.com/v1";
/// Domains of 1fichier links. Uploaders pick one, and a file only works on the domain it was
/// shared with, so links keep their domain.
const DOMAINS: [&str; 12] = [
    "1fichier.com",
    "alterupload.com",
    "cjoint.net",
    "desfichiers.com",
    "desfichiers.net",
    "dfichiers.com",
    "megadl.fr",
    "mesfichiers.org",
    "piecejointe.net",
    "pjointe.com",
    "tenvoi.com",
    "dl4free.com",
];
/// 3 requests per second at most.
const MIN_INTERVAL: Duration = Duration::from_millis(350);

pub const INFO: ProviderInfo = ProviderInfo {
    id: ID,
    name: "1fichier",
    website: "https://1fichier.com",
    api_key_url: "https://1fichier.com/console/params.pl",
    device_auth: false,
    magnets: false,
    kind: ProviderKind::Hoster,
    auth: AuthKind::ApiKey,
};

pub struct OneFichier {
    http: reqwest::Client,
    api_key: String,
    /// Earliest time of the next API call.
    next_call: Mutex<Instant>,
}

impl OneFichier {
    pub fn from_credentials(raw: &str, http: reqwest::Client) -> Result<Self, DebridError> {
        Ok(Self { http, api_key: api_key::decode(raw)?, next_call: Mutex::new(Instant::now()) })
    }

    /// POSTs a JSON request. Errors come as `{ "status": "KO", "message": "... #code" }`, usually
    /// with an HTTP error status.
    async fn call(&self, path: &str, body: Value) -> Result<Value, DebridError> {
        {
            let mut next_call = self.next_call.lock().await;
            tokio::time::sleep_until(*next_call).await;
            *next_call = Instant::now() + MIN_INTERVAL;
        }
        let response = self.http.post(format!("{API}{path}")).bearer_auth(&self.api_key).json(&body).send().await?;
        let status = response.status();
        let body: Value = response.json().await.unwrap_or(Value::Null);
        if status.is_success() && body.is_object() && body["status"] != "KO" {
            return Ok(body);
        }
        Err(map_error(status.as_u16(), body["message"].as_str().unwrap_or_default()))
    }
}

fn map_error(status: u16, message: &str) -> DebridError {
    let message =
        if message.trim().is_empty() { format!("1fichier answered HTTP {status}") } else { message.trim().to_owned() };
    let lower = message.to_ascii_lowercase();
    let has = |text: &str| lower.contains(text);
    if has("flood detected") || has("ip locked") || has("allowed at a time") || temporary_status(status) {
        DebridError::Temporary(message)
    } else if has("not authenticated") || has("no such user") || has("owner locked") || status == 401 {
        DebridError::Auth(message)
    } else if has("must be a customer") {
        DebridError::Quota(format!("{message} (the account is not premium)"))
    } else if has("password") {
        DebridError::LinkDown(format!("the file is password protected ({message})"))
    } else if has("resource not") || status == 404 {
        // "Resource not found" or "Resource not allowed" (private, restricted to some users).
        DebridError::LinkDown(message)
    } else {
        DebridError::Other(message)
    }
}

/// The only link form the API accepts: `https://<domain>/?<id>`. Legacy `https://<id>.1fichier.com/`
/// links are converted, extra parameters (`&af=...`) dropped.
fn file_link(link: &str) -> Option<String> {
    let url = url::Url::parse(link).ok()?;
    let host = url.host_str()?.to_ascii_lowercase();
    let host = host.strip_prefix("www.").unwrap_or(&host);
    if let Some(id) = host.strip_suffix(".1fichier.com").filter(|id| is_file_id(id)) {
        return Some(format!("https://1fichier.com/?{id}"));
    }
    if !DOMAINS.contains(&host) {
        return None;
    }
    let id = url.query()?.split('&').next()?.to_ascii_lowercase();
    is_file_id(&id).then(|| format!("https://{host}/?{id}"))
}

fn is_file_id(id: &str) -> bool {
    (5..=20).contains(&id.len()) && id.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit())
}

/// Reads `/user/info.cgi`. `offer` is 0 for accounts without subscription.
fn account_status(data: &Value, now: i64) -> AccountStatus {
    let end = data["subscription_end"].as_str().and_then(parse_french_time);
    let premium = lenient_i64(&data["offer"]).unwrap_or(0) > 0 && end.is_none_or(|end| end > now);
    AccountStatus {
        username: data["email"].as_str().map(str::to_owned),
        premium,
        premium_until: end.filter(|end| premium && *end > now),
        traffic_left: None,
    }
}

/// 1fichier dates (`YYYY-MM-DD HH:MM:SS`) are French local time: CET, or CEST from the last Sunday
/// of March to the last Sunday of October (01:00 UTC).
fn parse_french_time(text: &str) -> Option<i64> {
    let local = NaiveDateTime::parse_from_str(text.trim(), "%Y-%m-%d %H:%M:%S").ok()?;
    let summer_start = last_sunday(local.year(), 3)?.and_hms_opt(1, 0, 0)?;
    let summer_end = last_sunday(local.year(), 10)?.and_hms_opt(1, 0, 0)?;
    let as_summer_utc = local - chrono::Duration::hours(2);
    let offset = if (summer_start..summer_end).contains(&as_summer_utc) { 2 } else { 1 };
    Some((local - chrono::Duration::hours(offset)).and_utc().timestamp())
}

fn last_sunday(year: i32, month: u32) -> Option<NaiveDate> {
    let last_day = NaiveDate::from_ymd_opt(year, month + 1, 1)?.pred_opt()?;
    last_day.checked_sub_days(Days::new(u64::from(last_day.weekday().num_days_from_sunday())))
}

#[async_trait]
impl DebridService for OneFichier {
    fn info(&self) -> ProviderInfo {
        INFO
    }

    async fn account_status(&self) -> Result<AccountStatus, DebridError> {
        // Without parameters this only reads the account settings.
        let data = self.call("/user/info.cgi", json!({})).await?;
        Ok(account_status(&data, now()))
    }

    async fn supported_hosts(&self) -> Result<HostMatcher, DebridError> {
        Ok(HostMatcher::from_domains(&DOMAINS))
    }

    async fn unlock(&self, link: &str, password: Option<&str>) -> Result<Unlocked, DebridError> {
        let url = file_link(link).ok_or(DebridError::Unsupported)?;
        let mut body = json!({ "url": url });
        if let Some(password) = password {
            body["pass"] = json!(password);
        }
        let data = self.call("/download/get_token.cgi", body).await?;
        let url =
            data["url"].as_str().ok_or_else(|| DebridError::Other("1fichier returned no download link".into()))?;
        Ok(Unlocked { url: url.to_owned(), filename: None, size: None, hash: None })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_links() {
        assert_eq!(file_link("https://1fichier.com/?abc12345").as_deref(), Some("https://1fichier.com/?abc12345"));
        assert_eq!(
            file_link("http://www.alterupload.com/?AbC12345&af=99").as_deref(),
            Some("https://alterupload.com/?abc12345")
        );
        assert_eq!(file_link("https://abc12345.1fichier.com/").as_deref(), Some("https://1fichier.com/?abc12345"));
        assert_eq!(file_link("https://1fichier.com/dir/12345"), None);
        assert_eq!(file_link("https://1fichier.com/?ab"), None);
        assert_eq!(file_link("https://example.com/?abc12345"), None);
    }

    #[test]
    fn claims_every_domain() {
        let matcher = HostMatcher::from_domains(&DOMAINS);
        assert!(matcher.matches("https://1fichier.com/?abc12345"));
        assert!(matcher.matches("https://dl4free.com/?abc12345"));
        assert!(matcher.matches("https://abc12345.1fichier.com/"));
        assert!(!matcher.matches("https://rapidgator.net/file/abc"));
    }

    #[test]
    fn maps_errors() {
        let err = |status, message| map_error(status, message);
        assert!(matches!(err(403, "Flood detected: IP Locked #374"), DebridError::Temporary(_)));
        assert!(matches!(err(403, "Not authenticated #247"), DebridError::Auth(_)));
        assert!(matches!(err(403, "Must be a customer (Premium, Access) #200"), DebridError::Quota(_)));
        assert!(matches!(err(403, "Invalid password. Resource not allowed #631"), DebridError::LinkDown(_)));
        assert!(matches!(err(404, "Resource not found #469"), DebridError::LinkDown(_)));
        assert!(matches!(err(403, "Only 1 location allowed at a time #887"), DebridError::Temporary(_)));
        assert!(matches!(err(502, ""), DebridError::Temporary(_)));
        assert!(matches!(err(400, "Something new #1"), DebridError::Other(_)));
    }

    #[test]
    fn reads_account_info() {
        let now = parse_french_time("2026-10-15 12:00:00").unwrap();
        let data = json!({
            "email": "me@example.com",
            "offer": 1,
            "subscription_end": "2027-01-31 23:59:59",
            "cdn": 12.5,
        });
        let status = account_status(&data, now);
        assert!(status.premium);
        assert_eq!(status.username.as_deref(), Some("me@example.com"));
        assert_eq!(status.premium_until, parse_french_time("2027-01-31 23:59:59"));

        let expired = account_status(&json!({ "offer": 1, "subscription_end": "2020-01-01 00:00:00" }), now);
        assert!(!expired.premium);
        assert_eq!(expired.premium_until, None);
        assert!(!account_status(&json!({ "offer": 0, "email": "free@example.com" }), now).premium);
    }

    #[test]
    fn converts_french_time() {
        // Winter: UTC+1; summer: UTC+2.
        let winter = NaiveDate::from_ymd_opt(2026, 1, 10).unwrap().and_hms_opt(11, 0, 0).unwrap().and_utc();
        assert_eq!(parse_french_time("2026-01-10 12:00:00"), Some(winter.timestamp()));
        let summer = NaiveDate::from_ymd_opt(2026, 7, 10).unwrap().and_hms_opt(10, 0, 0).unwrap().and_utc();
        assert_eq!(parse_french_time("2026-07-10 12:00:00"), Some(summer.timestamp()));
        assert_eq!(last_sunday(2026, 3), NaiveDate::from_ymd_opt(2026, 3, 29));
        assert_eq!(last_sunday(2026, 10), NaiveDate::from_ymd_opt(2026, 10, 25));
        assert_eq!(parse_french_time("soon"), None);
    }
}
