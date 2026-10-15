//! Diagnostics report.

use axum::{
    Json,
    extract::State,
    http::{HeaderMap, header},
};
use serde_json::{Value, json};

use super::AppState;
use crate::diagnostics::{self, Context};

pub async fn get(State(state): State<AppState>, headers: HeaderMap) -> Json<Value> {
    let context = Context::new(&state.engine, behind_proxy(&headers), forwarded_https(&headers));
    Json(json!({ "checks": diagnostics::run(&context).await }))
}

/// Reverse proxies add one of these headers to the requests they forward.
fn behind_proxy(headers: &HeaderMap) -> bool {
    [header::FORWARDED.as_str(), "x-forwarded-for", "x-forwarded-proto", "x-forwarded-host", "x-real-ip"]
        .iter()
        .any(|name| headers.contains_key(*name))
}

/// Whether the proxy received the request over HTTPS.
fn forwarded_https(headers: &HeaderMap) -> bool {
    let value = |name: &str| headers.get(name).and_then(|v| v.to_str().ok()).map(str::to_ascii_lowercase);
    value("x-forwarded-proto").is_some_and(|proto| proto.split(',').next().is_some_and(|p| p.trim() == "https"))
        || value(header::FORWARDED.as_str()).is_some_and(|forwarded| {
            forwarded.split([';', ',']).any(|pair| pair.trim().replace(' ', "") == "proto=https")
        })
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderValue;

    use super::*;

    #[test]
    fn detects_reverse_proxies() {
        let mut headers = HeaderMap::new();
        assert!(!behind_proxy(&headers) && !forwarded_https(&headers));
        headers.insert("x-forwarded-for", HeaderValue::from_static("10.0.0.1"));
        assert!(behind_proxy(&headers) && !forwarded_https(&headers));
        headers.insert("x-forwarded-proto", HeaderValue::from_static("HTTPS"));
        assert!(forwarded_https(&headers));

        let mut headers = HeaderMap::new();
        headers.insert(header::FORWARDED, HeaderValue::from_static("for=10.0.0.1;proto=https;host=example.com"));
        assert!(behind_proxy(&headers) && forwarded_https(&headers));
        headers.insert(header::FORWARDED, HeaderValue::from_static("for=10.0.0.1;proto=http"));
        assert!(!forwarded_https(&headers));
    }
}
