//! HTTP API (`/api/...`) and web UI.

mod accounts;
mod auth;
mod backup;
mod categories;
mod diagnostics;
mod events;
mod history;
mod links;
mod logs;
mod media;
mod monitoring;
mod packages;
mod settings;
mod stats;
mod system;
mod telegram;
mod views;

use std::{
    collections::HashMap,
    net::SocketAddr,
    sync::{Arc, Mutex},
};

use axum::{
    Router,
    extract::DefaultBodyLimit,
    http::{HeaderValue, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use tower_http::{compression::CompressionLayer, set_header::SetResponseHeaderLayer};

use crate::{config::Config, engine::Engine, error::ApiError, logging::LogBuffer};

pub use auth::bootstrap_admin;

/// Shared state of every handler.
#[derive(Clone)]
pub struct AppState {
    pub engine: Arc<Engine>,
    pub config: Arc<Config>,
    pub logs: Arc<LogBuffer>,
    pub auth: Arc<auth::AuthState>,
    device_flows: Arc<Mutex<HashMap<String, accounts::DeviceFlow>>>,
}

impl AppState {
    pub fn new(engine: Arc<Engine>, config: Arc<Config>, logs: Arc<LogBuffer>) -> Self {
        Self { engine, config, logs, auth: Arc::default(), device_flows: Arc::default() }
    }

    fn db(&self) -> &crate::db::Db {
        &self.engine.db
    }
}

pub fn router(state: AppState) -> Router {
    let public = Router::new()
        .route("/api/auth/status", get(auth::status))
        .route("/api/auth/setup", post(auth::setup))
        .route("/api/auth/login", post(auth::login))
        .route("/api/auth/handoff", get(auth::handoff))
        .route("/api/health", get(monitoring::health));

    let protected = Router::new()
        .route("/api/auth/logout", post(auth::logout))
        .route("/api/auth/password", put(auth::change_password))
        .route("/api/system", get(system::info))
        .route("/api/queue/pause", post(system::pause))
        .route("/api/queue/resume", post(system::resume))
        .route("/api/events", get(events::stream))
        .route("/api/packages", get(packages::list).post(packages::add))
        .route("/api/packages/torrent", post(packages::add_torrent))
        .route("/api/packages/batch", post(packages::add_batch))
        .route("/api/links/check", post(links::check))
        .route("/api/packages/reorder", post(packages::reorder))
        .route("/api/packages/{id}", axum::routing::patch(packages::update).delete(packages::remove))
        .route("/api/packages/{id}/pause", post(packages::pause))
        .route("/api/packages/{id}/resume", post(packages::resume))
        .route("/api/packages/{id}/retry", post(packages::retry))
        .route("/api/packages/{id}/extract", post(packages::extract))
        .route("/api/packages/{id}/move", post(packages::move_to))
        .route("/api/downloads/{id}", delete(packages::remove_download))
        .route("/api/downloads/{id}/pause", post(packages::pause_download))
        .route("/api/downloads/{id}/resume", post(packages::resume_download))
        .route("/api/downloads/{id}/retry", post(packages::retry_download))
        .route("/api/history", get(history::list).delete(history::clear))
        .route("/api/history/{id}", delete(history::remove))
        .route("/api/history/{id}/redownload", post(history::redownload))
        .route("/api/categories", get(categories::list).post(categories::create))
        .route("/api/categories/reorder", post(categories::reorder))
        .route("/api/categories/{id}", put(categories::update).delete(categories::remove))
        .route("/api/accounts", get(accounts::list).post(accounts::create))
        .route("/api/accounts/reorder", post(accounts::reorder))
        .route("/api/accounts/{id}", put(accounts::update).delete(accounts::remove))
        .route("/api/accounts/{id}/check", post(accounts::check))
        .route("/api/accounts/device/{provider}/start", post(accounts::device_start))
        .route("/api/accounts/device/{provider}/poll", post(accounts::device_poll))
        .route("/api/settings", get(settings::get).put(settings::replace))
        .route("/api/settings/api-key", post(settings::regenerate_api_key))
        .route("/api/notifications/test", post(settings::test_notification))
        .route("/api/logs", get(logs::list).delete(logs::clear))
        .route("/api/logs/file", get(logs::file))
        .route("/api/stats", get(stats::get))
        .route("/api/backup", get(backup::download))
        .route("/api/backup/restore", post(backup::restore))
        .route("/api/integrations/telegram/test", post(telegram::test))
        .route("/api/system/diagnostics", get(diagnostics::get))
        .route("/api/media/status", get(media::status))
        .route("/api/media/install", post(media::install))
        .route("/api/media/probe", post(media::probe))
        .route_layer(middleware::from_fn_with_state(state.clone(), auth::require_auth));

    let api = Router::new().merge(public).merge(protected).layer(middleware::map_response(json_errors));
    Router::new()
        .merge(api)
        .route("/metrics", get(monitoring::metrics))
        .fallback(crate::web::serve)
        .layer(DefaultBodyLimit::max(32 * 1024 * 1024))
        .layer(CompressionLayer::new())
        .layer(SetResponseHeaderLayer::overriding(header::X_CONTENT_TYPE_OPTIONS, HeaderValue::from_static("nosniff")))
        .layer(SetResponseHeaderLayer::overriding(header::X_FRAME_OPTIONS, HeaderValue::from_static("SAMEORIGIN")))
        .layer(SetResponseHeaderLayer::overriding(header::REFERRER_POLICY, HeaderValue::from_static("same-origin")))
        .with_state(state)
}

/// Turns axum's plain-text rejections (malformed JSON, invalid path parameters...) into the
/// `{ "error", "code" }` shape every API error uses.
async fn json_errors(response: Response) -> Response {
    let plain_text = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.starts_with("text/plain"));
    let status = response.status();
    if !status.is_client_error() || !plain_text {
        return response;
    }
    let body = axum::body::to_bytes(response.into_body(), 64 * 1024).await.unwrap_or_default();
    let code = match status.as_u16() {
        404 => "NOT_FOUND",
        413 => "PAYLOAD_TOO_LARGE",
        415 => "UNSUPPORTED_MEDIA_TYPE",
        _ => "BAD_REQUEST",
    };
    ApiError::new(status, code, String::from_utf8_lossy(&body).trim()).into_response()
}

/// Opens the web interface's port.
pub async fn bind(address: SocketAddr) -> anyhow::Result<tokio::net::TcpListener> {
    tokio::net::TcpListener::bind(address).await.map_err(|e| anyhow::anyhow!("cannot listen on {address}: {e}"))
}

/// Serves the application until `shutdown` fires.
pub async fn serve(
    listener: tokio::net::TcpListener,
    state: AppState,
    shutdown: tokio_util::sync::CancellationToken,
) -> anyhow::Result<()> {
    if let Ok(address) = listener.local_addr() {
        tracing::info!("web interface on http://{address}");
    }
    axum::serve(listener, router(state).into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async move { shutdown.cancelled().await })
        .await?;
    Ok(())
}

/// `{ "ids": [..] }` body shared by reorder endpoints.
#[derive(serde::Deserialize)]
struct Ids {
    ids: Vec<i64>,
}
