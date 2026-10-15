//! Log history and log file download.

use axum::{
    Json,
    body::Body,
    extract::{Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::AppState;
use crate::{
    error::{ApiError, ApiResult},
    logging::{latest_log_file, level_from_name},
};

#[derive(Deserialize)]
pub struct LogQuery {
    #[serde(default = "default_limit")]
    limit: usize,
    #[serde(default = "default_level")]
    level: String,
}

fn default_limit() -> usize {
    500
}

fn default_level() -> String {
    "DEBUG".into()
}

pub async fn list(State(state): State<AppState>, Query(query): Query<LogQuery>) -> Json<Value> {
    let entries = state.logs.snapshot(level_from_name(&query.level), query.limit.clamp(1, 5_000));
    Json(json!({ "entries": entries }))
}

pub async fn clear(State(state): State<AppState>) -> StatusCode {
    state.logs.clear();
    StatusCode::NO_CONTENT
}

pub async fn file(State(state): State<AppState>) -> ApiResult<Response> {
    let path = latest_log_file(&state.config.logs_dir()).ok_or_else(|| ApiError::not_found("log file"))?;
    let file = tokio::fs::File::open(&path).await.map_err(ApiError::internal)?;
    let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    Ok((
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8".to_owned()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\"")),
        ],
        Body::from_stream(tokio_util::io::ReaderStream::new(file)),
    )
        .into_response())
}
