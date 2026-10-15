//! Telegram bot token check.

use axum::{Json, extract::State};
use serde::Deserialize;
use serde_json::{Value, json};

use super::AppState;
use crate::{
    error::{ApiError, ApiResult},
    integrations::telegram,
    settings::is_bot_token,
};

#[derive(Deserialize)]
pub struct TestRequest {
    /// Empty: the saved token.
    #[serde(default)]
    token: String,
}

pub async fn test(State(state): State<AppState>, Json(body): Json<TestRequest>) -> ApiResult<Json<Value>> {
    let token = match body.token.trim() {
        "" => state.engine.settings.get().telegram_bot.token.clone(),
        token => token.to_owned(),
    };
    if !is_bot_token(&token) {
        return Err(ApiError::bad_request(
            "INVALID_TOKEN",
            "this is not a bot token (it looks like `123456789:AAE...`)",
        ));
    }
    let username = telegram::bot_username(&state.engine.api_http, &token)
        .await
        .map_err(|error| ApiError::bad_request("TELEGRAM_ERROR", error.to_string()))?;
    Ok(Json(json!({ "username": username })))
}
