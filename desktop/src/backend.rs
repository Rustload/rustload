//! What the app drives: Rustload running inside it, or a Rustload server elsewhere (gateway mode).
//! Both give the window an address, take *Pause all* and report `stats` and `notice` events.

use std::{
    path::PathBuf,
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Context;
use futures_util::StreamExt;
use rustload::{Handle, Server};
use serde_json::Value;
use tauri::Url;
use tokio::sync::{broadcast::error::RecvError, mpsc};

/// An event of the server, as the interface receives it.
pub struct Update {
    pub name: String,
    pub data: Value,
}

/// Sent when the server cannot be reached, then again once it can.
pub const OFFLINE: &str = "offline";

pub enum Backend {
    Local { server: Mutex<Option<Server>>, handle: Handle },
    Remote(Remote),
}

impl Backend {
    pub fn local(server: Server) -> Self {
        let handle = server.handle();
        Self::Local { server: Mutex::new(Some(server)), handle }
    }

    /// The interface at `path` (`/`, `/?add=…`); the local one opens logged in.
    pub fn page(&self, path: &str) -> Option<Url> {
        match self {
            Self::Local { handle, .. } => handle.login_url(path).ok()?.parse().ok(),
            Self::Remote(remote) => remote.base.join(path.trim_start_matches('/')).ok(),
        }
    }

    /// Whether `url` belongs to the interface (other sites open in the browser).
    pub fn is_interface(&self, url: &Url) -> bool {
        match self {
            Self::Local { handle, .. } => {
                url.scheme() == "http"
                    && url.host_str() == Some("127.0.0.1")
                    && url.port() == Some(handle.address().port())
            }
            Self::Remote(remote) => url.origin() == remote.base.origin(),
        }
    }

    /// The downloads folder, when it is on this computer.
    pub fn download_root(&self) -> Option<PathBuf> {
        match self {
            Self::Local { handle, .. } => Some(handle.download_root().to_path_buf()),
            Self::Remote(_) => None,
        }
    }

    /// The server's name in the tooltip, for a remote one.
    pub fn label(&self) -> Option<String> {
        match self {
            Self::Local { .. } => None,
            Self::Remote(remote) => remote.base.host_str().map(str::to_owned),
        }
    }

    pub async fn set_paused(&self, paused: bool) -> anyhow::Result<()> {
        match self {
            Self::Local { handle, .. } => handle.set_paused(paused).await,
            Self::Remote(remote) => remote.set_paused(paused).await,
        }
    }

    /// The server's events from now on, until the app quits.
    pub fn updates(self: &Arc<Self>) -> mpsc::Receiver<Update> {
        let (tx, rx) = mpsc::channel(64);
        match &**self {
            Self::Local { handle, .. } => {
                let mut events = handle.events();
                tauri::async_runtime::spawn(async move {
                    loop {
                        let event = match events.recv().await {
                            Ok(event) => event,
                            Err(RecvError::Lagged(_)) => continue,
                            Err(RecvError::Closed) => return,
                        };
                        if tx.send(Update { name: event.name.to_owned(), data: event.data }).await.is_err() {
                            return;
                        }
                    }
                });
            }
            Self::Remote(_) => {
                let backend = self.clone();
                tauri::async_runtime::spawn(async move {
                    if let Self::Remote(remote) = &*backend {
                        remote.follow(tx).await;
                    }
                });
            }
        }
        rx
    }

    /// Stops downloading cleanly, for the local server.
    pub async fn shutdown(&self) {
        if let Self::Local { server, .. } = self {
            let server = server.lock().expect("poisoned").take();
            if let Some(server) = server {
                let _ = server.stop().await;
            }
        }
    }

    /// [`Backend::shutdown`] from the main thread, as the app ends.
    pub fn stop(&self) {
        tauri::async_runtime::block_on(self.shutdown());
    }
}

/// A Rustload server reached with an API key, like the browser extension.
pub struct Remote {
    base: Url,
    api_key: String,
    http: reqwest::Client,
}

impl Remote {
    pub fn new(base: Url, api_key: String) -> Self {
        Self { base, api_key, http: client() }
    }

    fn api(&self, path: &str) -> Url {
        self.base.join(path).expect("API paths are relative")
    }

    async fn set_paused(&self, paused: bool) -> anyhow::Result<()> {
        let path = if paused { "api/queue/pause" } else { "api/queue/resume" };
        self.http.post(self.api(path)).header("X-Api-Key", &self.api_key).send().await?.error_for_status()?;
        Ok(())
    }

    /// Forwards the server's Server-Sent Events, reconnecting with growing pauses while it is
    /// unreachable.
    async fn follow(&self, tx: mpsc::Sender<Update>) {
        let mut delay = Duration::from_secs(2);
        loop {
            let connected = self.stream(&tx).await;
            if tx.is_closed() {
                return;
            }
            let _ = tx.send(Update { name: OFFLINE.into(), data: Value::Null }).await;
            delay = if connected { Duration::from_secs(2) } else { (delay * 2).min(Duration::from_secs(30)) };
            tokio::time::sleep(delay).await;
        }
    }

    /// Reads the event stream until it ends. Returns whether it could connect.
    async fn stream(&self, tx: &mpsc::Sender<Update>) -> bool {
        let request = self
            .http
            .get(self.api("api/events"))
            .header("X-Api-Key", &self.api_key)
            .header("Accept", "text/event-stream")
            .send()
            .await
            .and_then(reqwest::Response::error_for_status);
        let Ok(response) = request else { return false };
        let mut body = response.bytes_stream();
        let mut parser = EventParser::default();
        while let Some(Ok(chunk)) = body.next().await {
            for update in parser.push(&chunk) {
                if tx.send(update).await.is_err() {
                    return true;
                }
            }
        }
        true
    }
}

/// Server-Sent Events, decoded line by line.
#[derive(Default)]
struct EventParser {
    buffer: Vec<u8>,
    name: String,
    data: Vec<String>,
}

impl EventParser {
    fn push(&mut self, bytes: &[u8]) -> Vec<Update> {
        self.buffer.extend_from_slice(bytes);
        let mut updates = Vec::new();
        while let Some(end) = self.buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=end).collect();
            let line = String::from_utf8_lossy(&line);
            let line = line.trim_end_matches(['\n', '\r']);
            if line.is_empty() {
                if !self.data.is_empty() {
                    let data = serde_json::from_str(&self.data.join("\n")).unwrap_or(Value::Null);
                    let name = if self.name.is_empty() { "message".into() } else { std::mem::take(&mut self.name) };
                    updates.push(Update { name, data });
                }
                self.name.clear();
                self.data.clear();
            } else if !line.starts_with(':') {
                let (field, value) = line.split_once(':').unwrap_or((line, ""));
                let value = value.strip_prefix(' ').unwrap_or(value);
                match field {
                    "event" => self.name = value.to_owned(),
                    "data" => self.data.push(value.to_owned()),
                    _ => {}
                }
            }
        }
        updates
    }
}

fn client() -> reqwest::Client {
    let _ = rustls::crypto::ring::default_provider().install_default();
    reqwest::Client::builder()
        .user_agent(concat!("Rustload-Desktop/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .build()
        .expect("the HTTP client builds")
}

/// Why a server cannot be used.
#[derive(Debug)]
pub enum CheckError {
    /// Nothing answers at this address.
    Unreachable,
    /// Something answers, but not Rustload.
    NotRustload,
    /// The server refuses the API key.
    BadKey,
}

/// Normalises what the person typed (`nas:8800` → `http://nas:8800/`) and checks that a Rustload
/// server answers there and accepts `api_key`. Returns the address and the server's version.
pub async fn check(address: &str, api_key: &str) -> Result<(Url, String), CheckError> {
    let address = address.trim();
    let address = if address.contains("://") { address.to_owned() } else { format!("http://{address}") };
    let mut base: Url = address.parse().map_err(|_| CheckError::Unreachable)?;
    if !matches!(base.scheme(), "http" | "https") || base.host_str().is_none() {
        return Err(CheckError::Unreachable);
    }
    if !base.path().ends_with('/') {
        base.set_path(&format!("{}/", base.path()));
    }
    base.set_query(None);
    base.set_fragment(None);

    let http = client();
    let health: Value = async {
        let response = http.get(base.join("api/health").ok()?).timeout(Duration::from_secs(10)).send().await.ok()?;
        response.json().await.ok()
    }
    .await
    .ok_or(CheckError::Unreachable)?;
    let version = health["version"].as_str().ok_or(CheckError::NotRustload)?.to_owned();
    let system = http
        .get(base.join("api/system").map_err(|_| CheckError::Unreachable)?)
        .header("X-Api-Key", api_key.trim())
        .timeout(Duration::from_secs(10))
        .send()
        .await
        .map_err(|_| CheckError::Unreachable)?;
    if !system.status().is_success() {
        return Err(CheckError::BadKey);
    }
    Ok((base, version))
}

/// Reads the address saved by an earlier run.
pub fn remote(url: &str, api_key: &str) -> anyhow::Result<Remote> {
    let base = url.parse().with_context(|| format!("invalid server address `{url}`"))?;
    Ok(Remote::new(base, api_key.to_owned()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_server_sent_events_split_anywhere() {
        let mut parser = EventParser::default();
        assert!(parser.push(b"event: stats\ndata: {\"spe").is_empty());
        let updates = parser.push(b"ed\": 5}\r\n\r\n: keep-alive\n\nevent: notice\ndata: {}\n\n");
        let names: Vec<&str> = updates.iter().map(|u| u.name.as_str()).collect();
        assert_eq!(names, ["stats", "notice"]);
        assert_eq!(updates[0].data["speed"], 5);
    }
}
