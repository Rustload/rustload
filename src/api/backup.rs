//! Backup download and restore.

use axum::{
    Json,
    body::Bytes,
    extract::{Query, State},
    http::header,
    response::{IntoResponse, Response},
};
use serde::Deserialize;

use super::AppState;
use crate::{
    backup::{self, RestoreSummary},
    error::{ApiError, ApiResult},
};

pub async fn download(State(state): State<AppState>) -> ApiResult<Response> {
    let settings = state.engine.settings.get();
    let backup = backup::export(state.db(), &settings, &state.config.config_dir).await?;
    let body = serde_json::to_vec_pretty(&backup).map_err(ApiError::internal)?;
    let name = format!("rustload-backup-{}.json", chrono::Local::now().format("%Y-%m-%d"));
    tracing::info!("backup downloaded");
    Ok((
        [
            (header::CONTENT_TYPE, "application/json".to_owned()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{name}\"")),
            // It contains credentials.
            (header::CACHE_CONTROL, "no-store".to_owned()),
        ],
        body,
    )
        .into_response())
}

#[derive(Deserialize)]
pub struct RestoreQuery {
    #[serde(default)]
    queue: bool,
}

pub async fn restore(
    State(state): State<AppState>,
    Query(query): Query<RestoreQuery>,
    body: Bytes,
) -> ApiResult<Json<RestoreSummary>> {
    let backup = backup::parse(&body).map_err(|error| ApiError::bad_request("INVALID_BACKUP", error.to_string()))?;
    Ok(Json(backup::restore(&state.engine, backup, query.queue).await?))
}
