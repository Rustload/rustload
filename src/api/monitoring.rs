//! Monitoring: Prometheus metrics (`/metrics`) and the health check (`/api/health`).

use std::{collections::HashMap, fmt::Write as _, time::Duration};

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use serde_json::json;

use super::{AppState, auth::is_api_key};
use crate::{
    db,
    engine::Engine,
    error::{ApiError, ApiResult},
    stats::Snapshot,
};

const VERSION: &str = env!("CARGO_PKG_VERSION");
const HEALTH_TIMEOUT: Duration = Duration::from_secs(3);
/// Prometheus text exposition format.
const CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// `GET /api/health`: public and cheap, for container health checks and uptime monitors.
pub async fn health(State(state): State<AppState>) -> Response {
    match tokio::time::timeout(HEALTH_TIMEOUT, db::ping(state.db())).await {
        Ok(Ok(())) => Json(json!({ "status": "ok", "version": VERSION })).into_response(),
        outcome => {
            let reason = match outcome {
                Ok(Err(error)) => format!("{error:#}"),
                _ => "timed out".to_owned(),
            };
            tracing::warn!("health check failed: the database does not answer ({reason})");
            let body = json!({
                "status": "error",
                "version": VERSION,
                "error": "the database does not answer",
                "code": "UNHEALTHY",
            });
            (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
        }
    }
}

/// `GET /metrics`, enabled in the settings. Scrapers authenticate with the API key, either as a
/// bearer token or in `X-Api-Key`.
pub async fn metrics(State(state): State<AppState>, headers: HeaderMap) -> ApiResult<Response> {
    let settings = state.engine.settings.get();
    if !settings.integrations.metrics {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "metrics are disabled"));
    }
    if state.config.auth_enabled && !presented_key(&headers).is_some_and(|key| is_api_key(&state, key)) {
        return Err(ApiError::unauthorized());
    }
    let body = render(&state.engine).await?;
    Ok(([(header::CONTENT_TYPE, CONTENT_TYPE)], body).into_response())
}

/// `Authorization: Bearer <key>` (what Prometheus sends) or `X-Api-Key: <key>`.
fn presented_key(headers: &HeaderMap) -> Option<&str> {
    let bearer = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().split_once(' '))
        .filter(|(scheme, _)| scheme.eq_ignore_ascii_case("bearer"))
        .map(|(_, token)| token.trim());
    bearer.or_else(|| headers.get("x-api-key").and_then(|v| v.to_str().ok()))
}

async fn render(engine: &Engine) -> anyhow::Result<String> {
    let snapshot = Snapshot::take(engine).await?;
    let counts: HashMap<String, i64> = db::stats::queue_status_counts(&engine.db).await?.into_iter().collect();
    let all_time = db::stats::all_time(&engine.db).await?;
    let accounts: Vec<_> = db::accounts::list(&engine.db).await?.into_iter().filter(|a| a.enabled).collect();
    let count = |status: &str| counts.get(status).copied().unwrap_or(0);

    let mut out = Exposition::default();
    out.gauge("rustload_up", "Whether Rustload is running.", 1i64);
    out.family("rustload_build_info", "gauge", "Rustload version.");
    out.sample("rustload_build_info", &[("version", VERSION)], 1i64);
    out.gauge("rustload_download_speed_bytes", "Current total download speed in bytes per second.", snapshot.speed);
    out.family("rustload_downloads", "gauge", "Files of unfinished packages by state (queued = waiting for a slot).");
    for (state, value) in [
        ("active", snapshot.active as i64),
        ("queued", snapshot.queued as i64),
        ("paused", count("paused")),
        ("failed", count("failed")),
    ] {
        out.sample("rustload_downloads", &[("state", state)], value);
    }
    out.gauge(
        "rustload_queue_remaining_bytes",
        "Bytes left to download in active and queued files.",
        snapshot.remaining,
    );
    if let Some(disk) = snapshot.disk {
        out.gauge("rustload_disk_free_bytes", "Free space of the download folder in bytes.", disk.free);
        out.gauge("rustload_disk_total_bytes", "Size of the download folder's file system in bytes.", disk.total);
    }
    out.counter("rustload_downloaded_bytes_total", "Bytes of completed files.", all_time.bytes);
    out.counter("rustload_files_completed_total", "Completed files.", all_time.files);
    out.counter("rustload_files_failed_total", "Files that failed for good.", all_time.failed);
    out.family(
        "rustload_account_traffic_left_bytes",
        "gauge",
        "Traffic left on a debrid account in bytes (accounts reporting a limit).",
    );
    // Accounts are named by provider and id: their labels often contain the e-mail address, and
    // API key clients cannot list accounts.
    for account in &accounts {
        if let Some(traffic) = account.traffic_left {
            let id = account.id.to_string();
            let labels = [("provider", account.provider.as_str()), ("account_id", id.as_str())];
            out.sample("rustload_account_traffic_left_bytes", &labels, traffic);
        }
    }
    out.family(
        "rustload_account_premium_until_seconds",
        "gauge",
        "End of the premium period of a debrid account, unix time.",
    );
    for account in &accounts {
        if let Some(until) = account.premium_until {
            let id = account.id.to_string();
            let labels = [("provider", account.provider.as_str()), ("account_id", id.as_str())];
            out.sample("rustload_account_premium_until_seconds", &labels, until);
        }
    }
    out.gauge("rustload_uptime_seconds", "Seconds since Rustload started.", engine.started_at.elapsed().as_secs());
    Ok(out.text)
}

/// Writer for the Prometheus text format (version 0.0.4).
#[derive(Default)]
struct Exposition {
    text: String,
}

impl Exposition {
    fn family(&mut self, name: &str, kind: &str, help: &str) {
        let _ = writeln!(self.text, "# HELP {name} {}", escape_help(help));
        let _ = writeln!(self.text, "# TYPE {name} {kind}");
    }

    fn sample(&mut self, name: &str, labels: &[(&str, &str)], value: impl Into<Value>) {
        self.text.push_str(name);
        if !labels.is_empty() {
            let labels: Vec<String> =
                labels.iter().map(|(key, value)| format!("{key}=\"{}\"", escape_label(value))).collect();
            let _ = write!(self.text, "{{{}}}", labels.join(","));
        }
        let _ = writeln!(self.text, " {}", value.into().0);
    }

    fn gauge(&mut self, name: &str, help: &str, value: impl Into<Value>) {
        self.family(name, "gauge", help);
        self.sample(name, &[], value);
    }

    fn counter(&mut self, name: &str, help: &str, value: impl Into<Value>) {
        self.family(name, "counter", help);
        self.sample(name, &[], value);
    }
}

/// Every metric is an integer; this accepts the integer types used for them.
struct Value(i128);

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Self(value.into())
    }
}

impl From<u64> for Value {
    fn from(value: u64) -> Self {
        Self(value.into())
    }
}

/// Label values escape backslashes, double quotes and line feeds.
fn escape_label(value: &str) -> String {
    value.replace('\\', r"\\").replace('"', "\\\"").replace('\n', r"\n")
}

/// Help texts escape backslashes and line feeds.
fn escape_help(text: &str) -> String {
    text.replace('\\', r"\\").replace('\n', r"\n")
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn escapes_label_values() {
        assert_eq!(escape_label(r#"My "main" C:\account"#), r#"My \"main\" C:\\account"#);
        assert_eq!(escape_label("two\nlines"), r"two\nlines");
        assert_eq!(escape_help("a\\b\nc \"d\""), "a\\\\b\\nc \"d\"");
    }

    #[test]
    fn writes_the_text_format() {
        let mut out = Exposition::default();
        out.gauge("rustload_up", "Whether Rustload is running.", 1i64);
        out.family("rustload_account_traffic_left_bytes", "gauge", "Traffic left.");
        out.sample("rustload_account_traffic_left_bytes", &[("provider", "alldebrid"), ("label", "Me \"2\"")], 5u64);
        out.counter("rustload_files_completed_total", "Completed files.", 42i64);
        assert_eq!(
            out.text,
            "# HELP rustload_up Whether Rustload is running.\n\
             # TYPE rustload_up gauge\n\
             rustload_up 1\n\
             # HELP rustload_account_traffic_left_bytes Traffic left.\n\
             # TYPE rustload_account_traffic_left_bytes gauge\n\
             rustload_account_traffic_left_bytes{provider=\"alldebrid\",label=\"Me \\\"2\\\"\"} 5\n\
             # HELP rustload_files_completed_total Completed files.\n\
             # TYPE rustload_files_completed_total counter\n\
             rustload_files_completed_total 42\n"
        );
    }

    #[test]
    fn reads_the_api_key_from_either_header() {
        let mut headers = HeaderMap::new();
        assert_eq!(presented_key(&headers), None);
        headers.insert("x-api-key", HeaderValue::from_static("key1"));
        assert_eq!(presented_key(&headers), Some("key1"));
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("bearer  key2"));
        assert_eq!(presented_key(&headers), Some("key2"));
        headers.insert(header::AUTHORIZATION, HeaderValue::from_static("Basic abc"));
        assert_eq!(presented_key(&headers), Some("key1"));
    }
}
