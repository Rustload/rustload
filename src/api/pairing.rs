//! Pairing the browser extension with the desktop app: the extension asks, the person answers in
//! the app, and the extension receives the address and API key it would otherwise be given by hand.

use std::{
    net::SocketAddr,
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
};
use futures_util::future::BoxFuture;
use serde::Deserialize;
use serde_json::json;

use super::AppState;
use crate::error::{ApiError, ApiResult};

/// How long the person has to answer in the app.
const ANSWER_TIMEOUT: Duration = Duration::from_secs(60);
/// Origins of browser extensions, the only pages allowed to ask.
const EXTENSION_SCHEMES: [&str; 3] = ["chrome-extension://", "moz-extension://", "safari-web-extension://"];

/// What the app shows the person before they allow the extension.
#[derive(Debug, Clone)]
pub struct PairRequest {
    /// `chrome-extension://…`, `moz-extension://…`.
    pub origin: String,
    /// The browser's name as the extension reports it (may be empty).
    pub browser: String,
}

/// Asks the person, resolves to their answer.
pub type Approver = Arc<dyn Fn(PairRequest) -> BoxFuture<'static, bool> + Send + Sync>;

#[derive(Default)]
pub struct Pairing {
    approver: RwLock<Option<Approver>>,
    /// A request waits for an answer.
    busy: AtomicBool,
}

impl Pairing {
    pub fn set_approver(&self, approver: Approver) {
        *self.approver.write().expect("poisoned") = Some(approver);
    }

    fn approver(&self) -> Option<Approver> {
        self.approver.read().expect("poisoned").clone()
    }
}

/// Clears `busy` however the request ends (answered, timed out, client gone).
struct Asking<'a>(&'a AtomicBool);

impl Drop for Asking<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

#[derive(Deserialize, Default)]
pub struct PairBody {
    #[serde(default)]
    browser: String,
}

/// The extension's origin, if the request comes from one.
fn extension_origin(headers: &HeaderMap) -> Option<HeaderValue> {
    let origin = headers.get(header::ORIGIN)?;
    let text = origin.to_str().ok()?;
    EXTENSION_SCHEMES.iter().any(|scheme| text.starts_with(scheme)).then(|| origin.clone())
}

/// Lets that origin (and only it) read the answer, whatever it is.
fn with_cors(origin: Option<HeaderValue>, response: impl IntoResponse) -> Response {
    let mut response = response.into_response();
    if let Some(origin) = origin {
        let headers = response.headers_mut();
        headers.insert(header::ACCESS_CONTROL_ALLOW_ORIGIN, origin);
        headers.insert(header::VARY, HeaderValue::from_static("Origin"));
    }
    response
}

/// `OPTIONS /api/desktop/pair`: the browser's CORS preflight for the JSON body.
pub async fn preflight(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let origin = extension_origin(&headers);
    if !state.config.desktop || origin.is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let mut response = with_cors(origin, StatusCode::NO_CONTENT);
    let headers = response.headers_mut();
    headers.insert(header::ACCESS_CONTROL_ALLOW_METHODS, HeaderValue::from_static("POST"));
    headers.insert(header::ACCESS_CONTROL_ALLOW_HEADERS, HeaderValue::from_static("content-type"));
    headers.insert(header::ACCESS_CONTROL_MAX_AGE, HeaderValue::from_static("600"));
    response
}

/// `POST /api/desktop/pair`: waits for the person's answer in the desktop app, then gives the
/// extension the interface's address and the API key.
pub async fn pair(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    body: Option<Json<PairBody>>,
) -> Response {
    let origin = extension_origin(&headers);
    let answer = ask(&state, peer, origin.as_ref(), body.map(|Json(body)| body).unwrap_or_default()).await;
    with_cors(origin, answer)
}

async fn ask(
    state: &AppState,
    peer: SocketAddr,
    origin: Option<&HeaderValue>,
    body: PairBody,
) -> ApiResult<Json<serde_json::Value>> {
    if !state.config.desktop {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "not found"));
    }
    let forbidden = |code, message| ApiError::new(StatusCode::FORBIDDEN, code, message);
    if !peer.ip().is_loopback() {
        return Err(forbidden("LOCAL_ONLY", "only this computer can pair with the app"));
    }
    let Some(origin) = origin.and_then(|o| o.to_str().ok()) else {
        return Err(forbidden("NOT_AN_EXTENSION", "only the browser extension can pair with the app"));
    };
    let approver = state.pairing.approver().ok_or_else(|| {
        ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "PAIRING_UNAVAILABLE", "the app cannot ask right now")
    })?;
    if state.pairing.busy.swap(true, Ordering::AcqRel) {
        return Err(ApiError::conflict("PAIRING_IN_PROGRESS", "the app is already asking: answer it first"));
    }
    let _asking = Asking(&state.pairing.busy);

    let browser: String = body.browser.trim().chars().take(40).collect();
    let request = PairRequest { origin: origin.to_owned(), browser };
    match tokio::time::timeout(ANSWER_TIMEOUT, approver(request.clone())).await {
        Err(_) => {
            return Err(ApiError::new(StatusCode::REQUEST_TIMEOUT, "PAIRING_TIMEOUT", "nobody answered in the app"));
        }
        Ok(false) => {
            tracing::info!("browser extension {} refused", request.origin);
            return Err(forbidden("PAIRING_REFUSED", "the connection was refused in the app"));
        }
        Ok(true) => {}
    }

    let settings = &state.engine.settings;
    let mut api_key = settings.get().integrations.api_key.clone();
    if api_key.is_empty() {
        api_key = settings.regenerate_api_key().await?;
    }
    tracing::info!("browser extension {} connected", request.origin);
    let port = state.address.port();
    Ok(Json(json!({ "api_key": api_key, "url": format!("http://127.0.0.1:{port}/") })))
}
