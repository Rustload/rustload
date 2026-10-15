//! Settings, API key and notification test.

use std::sync::Arc;

use axum::{Json, extract::State, http::StatusCode};
use serde_json::{Value, json};

use super::AppState;
use crate::{
    error::{ApiError, ApiResult},
    integrations::notify::Message,
    settings::{NotificationTarget, ReplaceError, Settings},
};

pub async fn get(State(state): State<AppState>) -> Json<Arc<Settings>> {
    Json(state.engine.settings.get())
}

impl From<ReplaceError> for ApiError {
    fn from(error: ReplaceError) -> Self {
        match error {
            ReplaceError::Invalid(message) => ApiError::bad_request("INVALID_SETTINGS", message),
            ReplaceError::Storage(error) => error.into(),
        }
    }
}

pub async fn replace(State(state): State<AppState>, Json(settings): Json<Settings>) -> ApiResult<Json<Arc<Settings>>> {
    let settings = state.engine.settings.replace(settings).await?;
    tracing::info!("settings updated");
    Ok(Json(settings))
}

pub async fn regenerate_api_key(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let api_key = state.engine.settings.regenerate_api_key().await?;
    tracing::info!("API key regenerated");
    Ok(Json(json!({ "api_key": api_key })))
}

pub async fn test_notification(
    State(state): State<AppState>,
    Json(target): Json<NotificationTarget>,
) -> ApiResult<StatusCode> {
    state
        .engine
        .notifier
        .deliver(&target, &Message::test())
        .await
        .map_err(|error| ApiError::bad_request("NOTIFICATION_FAILED", error))?;
    Ok(StatusCode::NO_CONTENT)
}
