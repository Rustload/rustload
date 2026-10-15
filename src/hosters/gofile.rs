//! Gofile folders (`/d/<id>`), through the API with a guest account.
//!
//! The API needs a guest token and a "website token" published in Gofile's front-end script;
//! downloads need the guest token as an `accountToken` cookie.

use std::{
    sync::OnceLock,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use regex::Regex;
use reqwest::header::{COOKIE, HeaderMap, HeaderValue};
use serde_json::Value;
use tokio::sync::Mutex;
use url::Url;

use super::{FolderEntry, FolderListing, Hoster, HosterError, HosterLink, host_is, segment_after, status_error};
use crate::util::join_relative;

const API: &str = "https://api.gofile.io";
/// Used when the current value cannot be read from Gofile's script.
const FALLBACK_WEBSITE_TOKEN: &str = "4fd6sg89d7s6";
const SESSION_TTL: Duration = Duration::from_secs(3600);

#[derive(Clone)]
struct Session {
    token: String,
    website_token: String,
    created: Instant,
}

#[derive(Default)]
pub struct Gofile {
    session: Mutex<Option<Session>>,
}

impl Gofile {
    async fn session(&self, http: &reqwest::Client) -> Result<Session, HosterError> {
        let mut cached = self.session.lock().await;
        if let Some(session) = cached.as_ref().filter(|s| s.created.elapsed() < SESSION_TTL) {
            return Ok(session.clone());
        }
        let account: Value = http.post(format!("{API}/accounts")).send().await?.json().await?;
        let token = account["data"]["token"]
            .as_str()
            .ok_or_else(|| HosterError::Temporary("Gofile did not create a guest account".into()))?
            .to_owned();
        let website_token = website_token(http).await.unwrap_or_else(|| FALLBACK_WEBSITE_TOKEN.to_owned());
        let session = Session { token, website_token, created: Instant::now() };
        *cached = Some(session.clone());
        Ok(session)
    }

    async fn contents(&self, http: &reqwest::Client, id: &str) -> Result<Value, HosterError> {
        let session = self.session(http).await?;
        let response = http
            .get(format!("{API}/contents/{id}"))
            .query(&[("wt", session.website_token.as_str())])
            .bearer_auth(&session.token)
            .send()
            .await?;
        if !response.status().is_success() {
            return Err(status_error(response.status()));
        }
        let body: Value = response.json().await?;
        match body["status"].as_str() {
            Some("ok") => Ok(body["data"].clone()),
            Some("error-notFound") => Err(HosterError::Offline("folder not found".into())),
            Some("error-passwordRequired") => Err(HosterError::Failed("this folder is password protected".into())),
            other => Err(HosterError::Temporary(format!("Gofile answered {}", other.unwrap_or("an error")))),
        }
    }

    async fn files(&self, http: &reqwest::Client, id: &str) -> Result<FolderListing, HosterError> {
        let mut entries = Vec::new();
        let mut name = None;
        let mut pending = vec![(id.to_owned(), String::new())];
        while let Some((id, prefix)) = pending.pop() {
            let data = self.contents(http, &id).await?;
            if name.is_none() {
                name = data["name"].as_str().map(str::to_owned);
            }
            for child in data["children"].as_object().into_iter().flat_map(|c| c.values()) {
                let name = child["name"].as_str().unwrap_or("file");
                match (child["type"].as_str(), child["id"].as_str(), child["link"].as_str()) {
                    (Some("folder"), Some(child_id), _) => {
                        let path = join_relative(&prefix, name);
                        pending.push((child_id.to_owned(), path));
                    }
                    (_, _, Some(link)) => entries.push(FolderEntry {
                        url: link.to_owned(),
                        name: Some(name.to_owned()),
                        size: child["size"].as_u64(),
                        rel_dir: prefix.clone(),
                    }),
                    _ => {}
                }
            }
        }
        Ok(FolderListing { name, entries })
    }
}

async fn website_token(http: &reqwest::Client) -> Option<String> {
    static TOKEN: OnceLock<Regex> = OnceLock::new();
    let regex = TOKEN.get_or_init(|| Regex::new(r#"wt\s*[:=]\s*["']([A-Za-z0-9]+)["']"#).expect("valid regex"));
    let script = http.get("https://gofile.io/dist/js/global.js").send().await.ok()?.text().await.ok()?;
    regex.captures(&script).map(|c| c[1].to_owned())
}

#[async_trait]
impl Hoster for Gofile {
    fn name(&self) -> &'static str {
        "gofile"
    }

    fn label(&self) -> &'static str {
        "Gofile"
    }

    fn matches(&self, url: &Url) -> bool {
        host_is(url, &["gofile.io"])
    }

    async fn expand(&self, http: &reqwest::Client, url: &Url) -> Result<Option<FolderListing>, HosterError> {
        match segment_after(url, "d") {
            Some(id) => self.files(http, id).await.map(Some),
            None => Ok(None),
        }
    }

    async fn resolve(&self, http: &reqwest::Client, url: &Url) -> Result<HosterLink, HosterError> {
        // A folder link added without expansion: take its first file.
        let entry = match segment_after(url, "d") {
            Some(id) => Some(
                self.files(http, id)
                    .await?
                    .entries
                    .into_iter()
                    .next()
                    .ok_or_else(|| HosterError::Offline("empty folder".into()))?,
            ),
            None => None,
        };
        let session = self.session(http).await?;
        let mut headers = HeaderMap::new();
        if let Ok(cookie) = HeaderValue::from_str(&format!("accountToken={}", session.token)) {
            headers.insert(COOKIE, cookie);
        }
        Ok(HosterLink {
            headers,
            filename: entry.as_ref().and_then(|e| e.name.clone()),
            size: entry.as_ref().and_then(|e| e.size),
            ..HosterLink::direct(entry.map(|e| e.url).unwrap_or_else(|| url.to_string()))
        })
    }
}
