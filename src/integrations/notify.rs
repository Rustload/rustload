//! Push notifications: generic webhook, Discord, Telegram, ntfy and Gotify.

use base64::Engine as _;
use serde_json::json;

use crate::{
    settings::{NotificationEvent, NotificationKind, NotificationTarget},
    util::human_bytes,
};

#[derive(Debug, Clone)]
pub struct Message {
    pub event: NotificationEvent,
    pub title: String,
    pub body: String,
    pub package: Option<String>,
}

impl Message {
    pub fn for_package(event: NotificationEvent, name: &str, size: u64, error: Option<&str>) -> Self {
        let (title, body) = match event {
            NotificationEvent::PackageFinished => {
                ("Download finished".to_owned(), format!("{name} ({})", human_bytes(size)))
            }
            NotificationEvent::PackageFailed => {
                ("Download failed".to_owned(), format!("{name}: {}", error.unwrap_or("unknown error")))
            }
            NotificationEvent::ExtractFailed => {
                ("Extraction failed".to_owned(), format!("{name}: {}", error.unwrap_or("unknown error")))
            }
            NotificationEvent::QueueFinished => ("Queue finished".to_owned(), "All downloads are finished".to_owned()),
            NotificationEvent::ChecksumFailed => {
                ("Checksum mismatch".to_owned(), format!("{name}: {}", error.unwrap_or("a file is corrupted")))
            }
            NotificationEvent::DiskLow => ("Low disk space".to_owned(), error.unwrap_or("downloads paused").to_owned()),
        };
        Self { event, title, body, package: Some(name.to_owned()) }
    }

    pub fn queue_finished() -> Self {
        Self::for_package(NotificationEvent::QueueFinished, "", 0, None).without_package()
    }

    pub fn disk_low(detail: &str) -> Self {
        Self::for_package(NotificationEvent::DiskLow, "", 0, Some(detail)).without_package()
    }

    pub fn test() -> Self {
        Self {
            event: NotificationEvent::PackageFinished,
            title: "Rustload".into(),
            body: "Test notification: everything is set up correctly.".into(),
            package: None,
        }
    }

    fn without_package(mut self) -> Self {
        self.package = None;
        self
    }

    fn is_failure(&self) -> bool {
        matches!(
            self.event,
            NotificationEvent::PackageFailed | NotificationEvent::ExtractFailed | NotificationEvent::ChecksumFailed
        )
    }
}

#[derive(Clone)]
pub struct Notifier {
    http: reqwest::Client,
}

impl Notifier {
    pub fn new(http: reqwest::Client) -> Self {
        Self { http }
    }

    /// Delivers `message` to every enabled target subscribed to `event`, in the background.
    pub fn send(&self, targets: &[NotificationTarget], event: NotificationEvent, message: Message) {
        for target in targets.iter().filter(|t| t.enabled && t.events.contains(&event)) {
            let (notifier, target, message) = (self.clone(), target.clone(), message.clone());
            tokio::spawn(async move {
                if let Err(error) = notifier.deliver(&target, &message).await {
                    tracing::warn!("notification `{}` failed: {error}", target.name);
                }
            });
        }
    }

    pub async fn deliver(&self, target: &NotificationTarget, message: &Message) -> Result<(), String> {
        let request = match target.kind {
            NotificationKind::Webhook => self.http.post(&target.url).json(&json!({
                "app": "rustload",
                "event": message.event,
                "title": message.title,
                "message": message.body,
                "package": message.package,
                "timestamp": crate::util::now(),
            })),
            NotificationKind::Discord => self.http.post(&target.url).json(&json!({
                "username": "Rustload",
                "embeds": [{
                    "title": message.title,
                    "description": message.body,
                    "color": if message.is_failure() { 0xD94F4F } else { 0x3FA34D },
                }],
            })),
            NotificationKind::Telegram => {
                self.http.post(format!("https://api.telegram.org/bot{}/sendMessage", target.token)).json(&json!({
                    "chat_id": target.chat_id,
                    "text": format!("<b>{}</b>\n{}", escape_html(&message.title), escape_html(&message.body)),
                    "parse_mode": "HTML",
                }))
            }
            NotificationKind::Ntfy => {
                let mut request = self
                    .http
                    .post(&target.url)
                    .header("Title", encode_header(&message.title))
                    .header("Tags", if message.is_failure() { "x" } else { "white_check_mark" })
                    .body(message.body.clone());
                if !target.token.is_empty() {
                    request = request.bearer_auth(&target.token);
                }
                request
            }
            NotificationKind::Gotify => self
                .http
                .post(format!("{}/message", target.url.trim_end_matches('/')))
                .query(&[("token", &target.token)])
                .json(&json!({ "title": message.title, "message": message.body, "priority": 5 })),
        };
        // Webhook and bot URLs embed tokens: keep them out of error messages.
        let response = request.send().await.map_err(|e| e.without_url().to_string())?;
        if response.status().is_success() {
            Ok(())
        } else {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            Err(format!("HTTP {status}: {}", body.chars().take(200).collect::<String>()))
        }
    }
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// HTTP headers must be ASCII: non-ASCII values use RFC 2047 encoding, which ntfy understands.
fn encode_header(value: &str) -> String {
    if value.is_ascii() {
        value.to_owned()
    } else {
        format!("=?UTF-8?B?{}?=", base64::engine::general_purpose::STANDARD.encode(value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_non_ascii_headers() {
        assert_eq!(encode_header("Done"), "Done");
        assert_eq!(encode_header("Été"), "=?UTF-8?B?w4l0w6k=?=");
    }

    #[test]
    fn builds_package_messages() {
        let message = Message::for_package(NotificationEvent::PackageFailed, "Movie", 0, Some("link offline"));
        assert_eq!(message.body, "Movie: link offline");
        assert!(message.is_failure());
    }
}
