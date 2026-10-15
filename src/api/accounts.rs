//! Accounts (debrid services and premium hosters), including the "connect with a code" device
//! flow.

use std::time::{Duration, Instant};

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{AppState, Ids};
use crate::{
    db::{self, Table},
    debrid::{self, AuthKind, DebridError, DevicePoll, ProviderInfo},
    error::{ApiError, ApiResult, OrNotFound},
    models::AccountRow,
    util::random_token,
};

#[derive(Serialize)]
pub struct AccountView {
    id: i64,
    provider: String,
    label: String,
    enabled: bool,
    position: i64,
    api_key_masked: String,
    username: Option<String>,
    premium: bool,
    premium_until: Option<i64>,
    traffic_left: Option<i64>,
    last_check: Option<i64>,
    last_error: Option<String>,
}

fn view(row: AccountRow) -> AccountView {
    let login = match auth_kind(&row.provider) {
        AuthKind::Credentials => debrid::login_username(&row.credentials),
        AuthKind::ApiKey => None,
    };
    AccountView {
        api_key_masked: debrid::masked_secret(&row.credentials),
        id: row.id,
        provider: row.provider,
        label: row.label,
        enabled: row.enabled,
        position: row.position,
        username: row.username.or(login),
        premium: row.premium,
        premium_until: row.premium_until,
        traffic_left: row.traffic_left,
        last_check: row.last_check,
        last_error: row.last_error,
    }
}

impl From<DebridError> for ApiError {
    fn from(error: DebridError) -> Self {
        match error {
            DebridError::Auth(message) => ApiError::bad_request("INVALID_API_KEY", message),
            other => ApiError::bad_request("DEBRID_ERROR", other.to_string()),
        }
    }
}

fn auth_kind(provider: &str) -> AuthKind {
    debrid::provider(provider).map_or(AuthKind::ApiKey, |info| info.auth)
}

/// Like `ApiError::from`, but rejected usernames/passwords get their own code.
fn rejected(provider: &str, error: DebridError) -> ApiError {
    match error {
        DebridError::Auth(message) if auth_kind(provider) == AuthKind::Credentials => {
            ApiError::bad_request("INVALID_CREDENTIALS", message)
        }
        other => other.into(),
    }
}

/// Secrets typed by the user; empty strings count as absent.
struct Secrets<'a> {
    api_key: Option<&'a str>,
    username: Option<&'a str>,
    password: Option<&'a str>,
}

impl<'a> Secrets<'a> {
    fn new(api_key: Option<&'a str>, username: Option<&'a str>, password: Option<&'a str>) -> Self {
        Self {
            api_key: api_key.map(str::trim).filter(|k| !k.is_empty()),
            username: username.map(str::trim).filter(|u| !u.is_empty()),
            // Spaces may be part of a password: only a fully blank one is absent.
            password: password.filter(|p| !p.trim().is_empty()),
        }
    }

    /// Refuses the fields the provider does not use.
    fn check_kind(&self, info: &ProviderInfo) -> ApiResult<()> {
        match info.auth {
            AuthKind::ApiKey if self.username.is_some() || self.password.is_some() => {
                Err(ApiError::bad_request("INVALID_API_KEY", format!("{} uses an API key", info.name)))
            }
            AuthKind::Credentials if self.api_key.is_some() => {
                Err(ApiError::bad_request("INVALID_CREDENTIALS", format!("{} uses a username and password", info.name)))
            }
            _ => Ok(()),
        }
    }

    /// Credentials JSON of a new account.
    fn credentials(&self, info: &ProviderInfo) -> ApiResult<String> {
        self.check_kind(info)?;
        match (info.auth, self.api_key, self.username, self.password) {
            (AuthKind::ApiKey, Some(key), _, _) => Ok(debrid::api_key_credentials(info.id, key)?),
            (AuthKind::ApiKey, None, _, _) => Err(ApiError::bad_request("INVALID_API_KEY", "the API key is required")),
            (AuthKind::Credentials, _, Some(username), Some(password)) => {
                Ok(debrid::login_credentials(info.id, username, password)?)
            }
            (AuthKind::Credentials, _, _, _) => {
                Err(ApiError::bad_request("INVALID_CREDENTIALS", "the username and password are required"))
            }
        }
    }

    /// New credentials JSON when some secret changed, `None` otherwise.
    fn changed_credentials(&self, info: &ProviderInfo, current: &str) -> ApiResult<Option<String>> {
        self.check_kind(info)?;
        Ok(match (info.auth, self.api_key) {
            (AuthKind::ApiKey, Some(key)) => Some(debrid::api_key_credentials(info.id, key)?),
            (AuthKind::ApiKey, None) => None,
            (AuthKind::Credentials, _) if self.username.is_some() || self.password.is_some() => {
                Some(debrid::update_login(current, self.username, self.password)?)
            }
            (AuthKind::Credentials, _) => None,
        })
    }
}

pub async fn list(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let accounts: Vec<AccountView> = db::accounts::list(state.db()).await?.into_iter().map(view).collect();
    Ok(Json(json!({ "accounts": accounts, "providers": debrid::providers() })))
}

async fn account_response(state: &AppState, id: i64) -> ApiResult<Json<AccountView>> {
    Ok(Json(view(db::accounts::get(state.db(), id).await?.or_not_found("account")?)))
}

#[derive(Deserialize)]
pub struct CreateAccount {
    provider: String,
    label: Option<String>,
    api_key: Option<String>,
    username: Option<String>,
    password: Option<String>,
}

pub async fn create(State(state): State<AppState>, Json(body): Json<CreateAccount>) -> ApiResult<Json<AccountView>> {
    let info = debrid::provider(&body.provider)
        .ok_or_else(|| ApiError::bad_request("UNKNOWN_PROVIDER", "unknown provider"))?;
    let secrets = Secrets::new(body.api_key.as_deref(), body.username.as_deref(), body.password.as_deref());
    let credentials = secrets.credentials(&info)?;
    let label = body.label.map(|l| l.trim().to_owned()).filter(|l| !l.is_empty());
    let id = add_account(&state, info.id, label.as_deref(), &credentials).await?;
    account_response(&state, id).await
}

/// Validates credentials against the provider, then stores the account.
async fn add_account(state: &AppState, provider: &str, label: Option<&str>, credentials: &str) -> ApiResult<i64> {
    let service = debrid::build(provider, credentials, state.engine.api_http.clone())?;
    let status = service.account_status().await.map_err(|error| rejected(provider, error))?;
    let info = service.info();
    let label = label.map(str::to_owned).unwrap_or_else(|| match &status.username {
        Some(username) => format!("{} ({username})", info.name),
        None => info.name.to_owned(),
    });
    let credentials = service.take_updated_credentials().unwrap_or_else(|| credentials.to_owned());
    let id = db::accounts::insert(state.db(), provider, &label, &credentials, true).await?;
    db::accounts::record_check(state.db(), id, Ok(&status)).await?;
    state.engine.debrid.reload().await?;
    tracing::info!("account `{label}` added");
    Ok(id)
}

#[derive(Deserialize)]
pub struct UpdateAccount {
    label: Option<String>,
    enabled: Option<bool>,
    api_key: Option<String>,
    username: Option<String>,
    password: Option<String>,
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<UpdateAccount>,
) -> ApiResult<Json<AccountView>> {
    let row = db::accounts::get(state.db(), id).await?.or_not_found("account")?;
    let info =
        debrid::provider(&row.provider).ok_or_else(|| ApiError::bad_request("UNKNOWN_PROVIDER", "unknown provider"))?;
    let secrets = Secrets::new(body.api_key.as_deref(), body.username.as_deref(), body.password.as_deref());
    let credentials = match secrets.changed_credentials(&info, &row.credentials)? {
        Some(credentials) => {
            // Refuse credentials the provider does not accept.
            let service = debrid::build(&row.provider, &credentials, state.engine.api_http.clone())?;
            service.account_status().await.map_err(|error| rejected(&row.provider, error))?;
            // Keep the session opened by the check, if any.
            Some(service.take_updated_credentials().unwrap_or(credentials))
        }
        None => None,
    };
    let label = body.label.as_deref().map(str::trim).filter(|l| !l.is_empty());
    db::accounts::update_settings(state.db(), id, label, body.enabled, credentials.as_deref()).await?;
    state.engine.debrid.reload().await?;
    if credentials.is_some() {
        let row = db::accounts::get(state.db(), id).await?.or_not_found("account")?;
        let _ = state.engine.debrid.check(&row).await;
    }
    account_response(&state, id).await
}

pub async fn remove(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let row = db::accounts::get(state.db(), id).await?.or_not_found("account")?;
    db::accounts::delete(state.db(), id).await?;
    state.engine.debrid.reload().await?;
    tracing::info!("account `{}` removed", row.label);
    Ok(StatusCode::NO_CONTENT)
}

pub async fn check(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<AccountView>> {
    let row = db::accounts::get(state.db(), id).await?.or_not_found("account")?;
    // The outcome (including errors) is stored on the account and returned below.
    let _ = state.engine.debrid.check(&row).await;
    account_response(&state, id).await
}

pub async fn reorder(State(state): State<AppState>, Json(body): Json<Ids>) -> ApiResult<StatusCode> {
    db::reorder(state.db(), Table::Accounts, &body.ids).await?;
    state.engine.debrid.reload().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// A pending device login, kept in memory until it completes or expires.
pub struct DeviceFlow {
    provider: String,
    state: Value,
    expires: Instant,
}

pub async fn device_start(State(state): State<AppState>, Path(provider): Path<String>) -> ApiResult<Json<Value>> {
    let info = debrid::provider(&provider)
        .filter(|p| p.device_auth)
        .ok_or_else(|| ApiError::bad_request("UNSUPPORTED", "this provider does not support connecting with a code"))?;
    let start = debrid::device_start(info.id, &state.engine.api_http).await?;
    let token = random_token();
    {
        let mut flows = state.device_flows.lock().expect("poisoned");
        flows.retain(|_, flow| flow.expires > Instant::now());
        flows.insert(
            token.clone(),
            DeviceFlow {
                provider: info.id.to_owned(),
                state: start.state.clone(),
                expires: Instant::now() + Duration::from_secs(start.expires_in),
            },
        );
    }
    Ok(Json(json!({
        "user_code": start.user_code,
        "verification_url": start.verification_url,
        "expires_in": start.expires_in,
        "poll_token": token,
    })))
}

#[derive(Deserialize)]
pub struct PollRequest {
    poll_token: String,
}

pub async fn device_poll(
    State(state): State<AppState>,
    Path(provider): Path<String>,
    Json(body): Json<PollRequest>,
) -> ApiResult<Json<Value>> {
    let flow = {
        let flows = state.device_flows.lock().expect("poisoned");
        flows.get(&body.poll_token).filter(|f| f.provider == provider).map(|f| (f.state.clone(), f.expires))
    };
    let Some((flow_state, expires)) = flow else {
        return Ok(Json(json!({ "status": "expired", "account": null })));
    };
    if expires <= Instant::now() {
        state.device_flows.lock().expect("poisoned").remove(&body.poll_token);
        return Ok(Json(json!({ "status": "expired", "account": null })));
    }
    match debrid::device_poll(&provider, &state.engine.api_http, &flow_state).await? {
        DevicePoll::Pending => Ok(Json(json!({ "status": "pending", "account": null }))),
        DevicePoll::Expired => {
            state.device_flows.lock().expect("poisoned").remove(&body.poll_token);
            Ok(Json(json!({ "status": "expired", "account": null })))
        }
        DevicePoll::Activated(credentials) => {
            state.device_flows.lock().expect("poisoned").remove(&body.poll_token);
            let id = add_account(&state, &provider, None, &credentials).await?;
            let account = view(db::accounts::get(state.db(), id).await?.or_not_found("account")?);
            Ok(Json(json!({ "status": "activated", "account": account })))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: &str) -> ProviderInfo {
        debrid::provider(id).unwrap()
    }

    #[test]
    fn requires_the_fields_of_the_provider() {
        let key = Secrets::new(Some(" abc "), None, Some(""));
        assert!(key.credentials(&info("alldebrid")).unwrap().contains("\"abc\""));
        assert!(key.credentials(&info("rapidgator")).is_err());

        let login = Secrets::new(None, Some("me@example.com"), Some(" secret"));
        assert!(login.credentials(&info("rapidgator")).is_ok());
        assert!(login.credentials(&info("1fichier")).is_err());

        let partial = Secrets::new(Some(""), Some("me@example.com"), Some("   "));
        assert!(partial.credentials(&info("rapidgator")).is_err());
        assert!(Secrets::new(None, None, None).credentials(&info("1fichier")).is_err());
    }

    #[test]
    fn updates_only_what_changed() {
        let current = debrid::login_credentials("rapidgator", "old@example.com", "secret").unwrap();
        let none = Secrets::new(None, Some(" "), None);
        assert!(none.changed_credentials(&info("rapidgator"), &current).unwrap().is_none());

        let password = Secrets::new(None, None, Some("new secret"));
        let updated = password.changed_credentials(&info("rapidgator"), &current).unwrap().unwrap();
        assert_eq!(debrid::login_username(&updated).as_deref(), Some("old@example.com"));
        assert!(updated.contains("new secret"));

        let key = Secrets::new(Some("key"), None, None);
        assert!(key.changed_credentials(&info("rapidgator"), &current).is_err());
        assert!(key.changed_credentials(&info("torbox"), "{}").unwrap().is_some());
    }
}
