//! Login, sessions and the authentication middleware.
//!
//! Sessions are random tokens kept in an HttpOnly cookie; only their SHA-256 is stored. Tools can
//! authenticate with the `X-Api-Key` header instead.

use std::{
    collections::HashMap,
    net::{IpAddr, SocketAddr},
    sync::Mutex,
    time::{Duration, Instant},
};

use argon2::{Argon2, PasswordHasher, PasswordVerifier};
use axum::{
    Extension, Json,
    extract::{ConnectInfo, Query, Request, State},
    http::{HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Redirect, Response},
};
use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use serde::Deserialize;
use serde_json::json;

use super::AppState;
use crate::{
    db,
    error::{ApiError, ApiResult},
    util::{now, random_token, sha256_hex},
};

const COOKIE: &str = "rustload_session";
const REMEMBER_TTL: i64 = 30 * 24 * 3600;
const SESSION_TTL: i64 = 24 * 3600;
const MIN_PASSWORD_LEN: usize = 8;
/// Failed logins allowed per client address and window before answering 429.
const MAX_FAILURES: u32 = 20;
const FAILURE_WINDOW: Duration = Duration::from_secs(15 * 60);

/// How long the desktop app's login addresses stay valid.
const HANDOFF_TTL: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct AuthState {
    failures: Mutex<HashMap<IpAddr, (u32, Instant)>>,
    /// SHA-256 of the single-use codes issued to the desktop app, with their creation time.
    handoffs: Mutex<Vec<(String, Instant)>>,
}

impl AuthState {
    /// A new single-use code for `/api/auth/handoff`.
    pub fn issue_handoff(&self) -> String {
        let code = random_token();
        let mut handoffs = self.handoffs.lock().expect("poisoned");
        handoffs.retain(|(_, issued)| issued.elapsed() < HANDOFF_TTL);
        handoffs.push((sha256_hex(&code), Instant::now()));
        code
    }

    /// Whether `code` was issued and not used yet; it cannot be used again.
    fn take_handoff(&self, code: &str) -> bool {
        let hash = sha256_hex(code);
        let mut handoffs = self.handoffs.lock().expect("poisoned");
        handoffs.retain(|(_, issued)| issued.elapsed() < HANDOFF_TTL);
        let before = handoffs.len();
        handoffs.retain(|(issued, _)| !constant_time_eq(issued.as_bytes(), hash.as_bytes()));
        handoffs.len() < before
    }

    fn blocked(&self, ip: IpAddr) -> bool {
        let failures = self.failures.lock().expect("poisoned");
        failures.get(&ip).is_some_and(|(count, since)| *count >= MAX_FAILURES && since.elapsed() < FAILURE_WINDOW)
    }

    fn record_failure(&self, ip: IpAddr) {
        let mut failures = self.failures.lock().expect("poisoned");
        if failures.len() > 1024 {
            failures.retain(|_, (_, since)| since.elapsed() < FAILURE_WINDOW);
        }
        let entry = failures.entry(ip).or_insert((0, Instant::now()));
        if entry.1.elapsed() >= FAILURE_WINDOW {
            *entry = (0, Instant::now());
        }
        entry.0 += 1;
    }

    fn clear(&self, ip: IpAddr) {
        self.failures.lock().expect("poisoned").remove(&ip);
    }
}

/// The authenticated caller, available to handlers as an extension.
#[derive(Debug, Clone, Default)]
pub struct Caller {
    pub user_id: Option<i64>,
    pub token_hash: Option<String>,
    /// Authenticated with the API key rather than a session.
    pub api_key: bool,
}

/// Endpoints the API key cannot use: they manage credentials or run commands (hooks), which
/// requires a logged-in user.
const SESSION_ONLY: [&str; 10] = [
    "/api/settings",
    "/api/accounts",
    "/api/notifications",
    "/api/auth/",
    "/api/backup",
    "/api/integrations/",
    "/api/system/diagnostics",
    "/api/media/install",
    "/api/media/ffmpeg/install",
    "/api/extraction/",
];
/// Header the web interface adds to every request. Other sites cannot set it without a CORS
/// preflight, which is never granted: requests without it are refused (CSRF protection).
const CSRF_HEADER: &str = "x-rustload-request";

pub async fn require_auth(State(state): State<AppState>, jar: CookieJar, mut request: Request, next: Next) -> Response {
    // `Request` is not `Sync`: copy what is needed before awaiting.
    let api_key = request.headers().get("x-api-key").and_then(|v| v.to_str().ok()).map(str::to_owned);
    let path = request.uri().path().to_owned();
    let unsafe_method = !matches!(*request.method(), Method::GET | Method::HEAD | Method::OPTIONS);
    let has_csrf_header = request.headers().contains_key(CSRF_HEADER);

    let caller = match authenticate(&state, &jar, api_key.as_deref()).await {
        Ok(caller) => caller,
        Err(error) => return error.into_response(),
    };
    if caller.api_key && SESSION_ONLY.iter().any(|prefix| path.starts_with(prefix)) {
        return ApiError::new(StatusCode::FORBIDDEN, "SESSION_REQUIRED", "this endpoint requires a logged-in user")
            .into_response();
    }
    if !caller.api_key && unsafe_method && !has_csrf_header {
        return ApiError::new(StatusCode::FORBIDDEN, "CSRF", "missing X-Rustload-Request header").into_response();
    }
    // The desktop app's window stays open for weeks and has no password to sign in again with:
    // its session moves forward while it is used.
    let renewed = match (&caller.token_hash, state.config.desktop) {
        (Some(token_hash), true) => {
            let now = now();
            db::users::extend_session(state.db(), token_hash, now + REMEMBER_TTL, now + REMEMBER_TTL / 2)
                .await
                .unwrap_or(false)
        }
        _ => false,
    };
    request.extensions_mut().insert(caller);
    let response = next.run(request).await;
    match jar.get(COOKIE).map(|cookie| cookie.value().to_owned()) {
        Some(token) if renewed => (jar.add(session_cookie(&state, token, true)), response).into_response(),
        _ => response,
    }
}

async fn authenticate(state: &AppState, jar: &CookieJar, api_key: Option<&str>) -> ApiResult<Caller> {
    if !state.config.auth_enabled {
        return Ok(Caller::default());
    }
    if let Some(key) = api_key {
        if is_api_key(state, key) {
            return Ok(Caller { api_key: true, ..Caller::default() });
        }
        return Err(ApiError::unauthorized());
    }
    if let Some(cookie) = jar.get(COOKIE) {
        let token_hash = sha256_hex(cookie.value());
        if let Some(user) = db::users::session_user(state.db(), &token_hash).await? {
            return Ok(Caller { user_id: Some(user.id), token_hash: Some(token_hash), api_key: false });
        }
    }
    Err(ApiError::unauthorized())
}

/// Whether `key` is the API key of the settings; never true while none is set.
pub(super) fn is_api_key(state: &AppState, key: &str) -> bool {
    let settings = state.engine.settings.get();
    let expected = &settings.integrations.api_key;
    !expected.is_empty() && constant_time_eq(key.as_bytes(), expected.as_bytes())
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

pub async fn status(State(state): State<AppState>, jar: CookieJar) -> ApiResult<Json<serde_json::Value>> {
    let setup_required = state.config.auth_enabled && db::users::count(state.db()).await? == 0;
    let user = match jar.get(COOKIE) {
        Some(cookie) if state.config.auth_enabled => db::users::session_user(state.db(), &sha256_hex(cookie.value()))
            .await?
            .map(|u| json!({ "username": u.username })),
        _ => None,
    };
    Ok(Json(json!({
        "auth_enabled": state.config.auth_enabled,
        "setup_required": setup_required,
        "user": user,
        "desktop": state.config.desktop,
    })))
}

#[derive(Deserialize)]
pub struct Handoff {
    code: String,
    #[serde(default)]
    next: String,
}

/// Opens the desktop app's window (or a browser it launched) logged in, with a code only the app
/// knows. The interface answers on this computer only and the app's account has a random password.
pub async fn handoff(
    State(state): State<AppState>,
    jar: CookieJar,
    Query(query): Query<Handoff>,
) -> ApiResult<Response> {
    if !state.config.desktop {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "NOT_FOUND", "not found"));
    }
    if !state.auth.take_handoff(&query.code) {
        return Err(ApiError::unauthorized());
    }
    // Only paths of this site (`//host` or `/\host` would leave it) that fit in a header.
    let next = Some(query.next.as_str())
        .filter(|next| next.starts_with('/') && !next.starts_with("//") && !next.starts_with("/\\"))
        .filter(|next| HeaderValue::from_str(next).is_ok())
        .unwrap_or("/");
    let redirect = Redirect::to(next);
    // Always a fresh session, replacing the window's previous one, which may be about to expire.
    if let Some(cookie) = jar.get(COOKIE) {
        db::users::delete_session(state.db(), &sha256_hex(cookie.value())).await?;
    }
    let user = db::users::first(state.db()).await?.ok_or_else(ApiError::unauthorized)?;
    let cookie = new_session_cookie(&state, user.id, true).await?;
    Ok((jar.add(cookie), redirect).into_response())
}

#[derive(Deserialize)]
pub struct Credentials {
    username: String,
    password: String,
    #[serde(default)]
    remember: bool,
}

pub async fn setup(
    State(state): State<AppState>,
    jar: CookieJar,
    Json(body): Json<Credentials>,
) -> ApiResult<Response> {
    let already = || ApiError::conflict("ALREADY_SETUP", "an administrator account already exists");
    if db::users::count(state.db()).await? > 0 {
        return Err(already());
    }
    let username = validate_username(&body.username)?;
    validate_password(&body.password)?;
    // Atomic: two concurrent setups cannot both create an account.
    let id = db::users::insert_first(state.db(), &username, &hash_password(body.password).await?)
        .await?
        .ok_or_else(already)?;
    tracing::info!("administrator account `{username}` created");
    open_session(&state, jar, id, &username, true).await
}

pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    jar: CookieJar,
    Json(body): Json<Credentials>,
) -> ApiResult<Response> {
    let ip = peer.ip();
    if state.auth.blocked(ip) {
        return Err(ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "TOO_MANY_ATTEMPTS",
            "too many failed logins, try again later",
        ));
    }
    let user = db::users::find_by_name(state.db(), body.username.trim()).await?;
    let valid = match &user {
        Some(user) => verify_password(body.password, user.password_hash.clone()).await,
        None => false,
    };
    match user {
        Some(user) if valid => {
            state.auth.clear(ip);
            tracing::info!("`{}` logged in from {ip}", user.username);
            open_session(&state, jar, user.id, &user.username, body.remember).await
        }
        _ => {
            state.auth.record_failure(ip);
            tracing::warn!("failed login for `{}` from {ip}", body.username.trim());
            // Slows down guessing without blocking legitimate users for long.
            tokio::time::sleep(Duration::from_secs(1)).await;
            Err(ApiError::new(StatusCode::UNAUTHORIZED, "INVALID_CREDENTIALS", "invalid username or password"))
        }
    }
}

async fn open_session(
    state: &AppState,
    jar: CookieJar,
    user_id: i64,
    username: &str,
    remember: bool,
) -> ApiResult<Response> {
    let cookie = new_session_cookie(state, user_id, remember).await?;
    Ok((jar.add(cookie), Json(json!({ "user": { "username": username } }))).into_response())
}

async fn new_session_cookie(state: &AppState, user_id: i64, remember: bool) -> ApiResult<Cookie<'static>> {
    let token = random_token();
    let ttl = if remember { REMEMBER_TTL } else { SESSION_TTL };
    db::users::insert_session(state.db(), &sha256_hex(&token), user_id, now() + ttl).await?;
    let _ = db::users::purge_expired_sessions(state.db()).await;
    Ok(session_cookie(state, token, remember))
}

fn session_cookie(state: &AppState, token: String, remember: bool) -> Cookie<'static> {
    let mut cookie = Cookie::build((COOKIE, token))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Strict)
        .secure(state.config.secure_cookies);
    if remember {
        cookie = cookie.max_age(time_duration(REMEMBER_TTL));
    }
    cookie.build()
}

/// Cookies use the `time` crate's duration type.
fn time_duration(seconds: i64) -> cookie::time::Duration {
    cookie::time::Duration::seconds(seconds)
}

pub async fn logout(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    jar: CookieJar,
) -> ApiResult<Response> {
    if let Some(token_hash) = &caller.token_hash {
        db::users::delete_session(state.db(), token_hash).await?;
    }
    let removal = Cookie::build((COOKIE, ""))
        .path("/")
        .http_only(true)
        .same_site(SameSite::Strict)
        .secure(state.config.secure_cookies)
        .build();
    Ok((jar.remove(removal), StatusCode::NO_CONTENT).into_response())
}

#[derive(Deserialize)]
pub struct PasswordChange {
    /// Not asked by the desktop app, whose account starts with a random password nobody knows.
    #[serde(default)]
    current_password: String,
    new_password: String,
}

pub async fn change_password(
    State(state): State<AppState>,
    Extension(caller): Extension<Caller>,
    Json(body): Json<PasswordChange>,
) -> ApiResult<StatusCode> {
    let user_id = caller
        .user_id
        .ok_or_else(|| ApiError::bad_request("NO_USER", "log in with a user account to change the password"))?;
    let user = db::users::get(state.db(), user_id).await?.ok_or_else(ApiError::unauthorized)?;
    if !state.config.desktop && !verify_password(body.current_password, user.password_hash).await {
        return Err(ApiError::bad_request("INVALID_CREDENTIALS", "the current password is incorrect"));
    }
    validate_password(&body.new_password)?;
    db::users::set_password(state.db(), user_id, &hash_password(body.new_password).await?).await?;
    if let Some(token_hash) = &caller.token_hash {
        db::users::delete_other_sessions(state.db(), user_id, token_hash).await?;
    }
    tracing::info!("password of `{}` changed", user.username);
    Ok(StatusCode::NO_CONTENT)
}

fn validate_username(username: &str) -> ApiResult<String> {
    let username = username.trim();
    if username.is_empty() || username.len() > 64 {
        return Err(ApiError::bad_request("INVALID_USERNAME", "the username must contain 1 to 64 characters"));
    }
    Ok(username.to_owned())
}

fn validate_password(password: &str) -> ApiResult<()> {
    if password.chars().count() < MIN_PASSWORD_LEN {
        return Err(ApiError::bad_request(
            "WEAK_PASSWORD",
            format!("the password must contain at least {MIN_PASSWORD_LEN} characters"),
        ));
    }
    Ok(())
}

/// Argon2 is deliberately slow: run it off the async workers.
pub async fn hash_password(password: String) -> ApiResult<String> {
    tokio::task::spawn_blocking(move || Argon2::default().hash_password(password.as_bytes()).map(|h| h.to_string()))
        .await
        .map_err(ApiError::internal)?
        .map_err(ApiError::internal)
}

async fn verify_password(password: String, hash: String) -> bool {
    tokio::task::spawn_blocking(move || Argon2::default().verify_password(password.as_bytes(), hash.as_str()).is_ok())
        .await
        .unwrap_or(false)
}

/// Creates the administrator from `RUSTLOAD_ADMIN_USER`/`RUSTLOAD_ADMIN_PASSWORD` on first start.
pub async fn bootstrap_admin(db: &db::Db, username: Option<&str>, password: Option<&str>) -> anyhow::Result<()> {
    let (Some(username), Some(password)) = (username, password) else { return Ok(()) };
    if db::users::count(db).await? > 0 {
        return Ok(());
    }
    let username = validate_username(username).map_err(|_| anyhow::anyhow!("invalid RUSTLOAD_ADMIN_USER"))?;
    validate_password(password).map_err(|_| anyhow::anyhow!("RUSTLOAD_ADMIN_PASSWORD is too short"))?;
    let hash =
        hash_password(password.to_owned()).await.map_err(|_| anyhow::anyhow!("cannot hash the admin password"))?;
    db::users::insert(db, &username, &hash).await?;
    tracing::info!("administrator account `{username}` created");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn hashes_and_verifies_passwords() {
        let hash = hash_password("correct horse".into()).await.unwrap();
        assert!(hash.starts_with("$argon2"));
        assert!(verify_password("correct horse".into(), hash.clone()).await);
        assert!(!verify_password("wrong".into(), hash).await);
    }

    #[test]
    fn compares_in_constant_time() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }
}
