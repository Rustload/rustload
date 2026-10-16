//! Media sites: yt-dlp status and installation, link probing.

use axum::{Json, extract::State, http::StatusCode};
use serde::Deserialize;

use super::AppState;
use crate::{
    db,
    error::{ApiError, ApiResult},
    media::{self, MediaError, MediaInfo, MediaStatus, tools::InstallError},
    util::now,
};

pub async fn status(State(state): State<AppState>) -> ApiResult<Json<MediaStatus>> {
    Ok(Json(media::status(&state.engine).await?))
}

/// Downloads the latest official yt-dlp into the config folder (session only).
pub async fn install(State(state): State<AppState>) -> ApiResult<Json<MediaStatus>> {
    match state.engine.media.install_latest().await {
        Ok(version) => {
            tracing::info!("yt-dlp {version} installed from the interface");
            // The latest release was just fetched: that counts as an update check.
            db::kv_set(state.db(), media::UPDATE_CHECK_KEY, &now().to_string()).await?;
        }
        Err(error) => return Err(install_error("yt-dlp", error)),
    }
    Ok(Json(media::status(&state.engine).await?))
}

/// Downloads yt-dlp's FFmpeg build into the config folder, on Windows (session only).
pub async fn install_ffmpeg(State(state): State<AppState>) -> ApiResult<Json<MediaStatus>> {
    match state.engine.media.install_ffmpeg().await {
        Ok(version) => tracing::info!("ffmpeg {version} installed from the interface"),
        Err(error) => return Err(install_error("ffmpeg", error)),
    }
    Ok(Json(media::status(&state.engine).await?))
}

/// The API error of a failed tool installation.
pub(super) fn install_error(tool: &str, error: InstallError) -> ApiError {
    tracing::warn!("{tool} installation failed: {error}");
    match error {
        InstallError::Busy => ApiError::conflict("UPDATE_IN_PROGRESS", error.to_string()),
        InstallError::Unsupported { .. } => ApiError::bad_request("UNSUPPORTED_PLATFORM", error.to_string()),
        _ => ApiError::new(StatusCode::BAD_GATEWAY, "INSTALL_FAILED", error.to_string()),
    }
}

#[derive(Deserialize)]
pub struct ProbeRequest {
    url: String,
}

pub async fn probe(State(state): State<AppState>, Json(body): Json<ProbeRequest>) -> ApiResult<Json<MediaInfo>> {
    media::probe(&state.engine, body.url.trim(), media::PROBE_TIMEOUT).await.map(Json).map_err(|error| match error {
        MediaError::Disabled => ApiError::conflict("MEDIA_DISABLED", error.to_string()),
        MediaError::Missing => ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "YTDLP_MISSING", error.to_string()),
        MediaError::Unavailable(_) | MediaError::Timeout(_) | MediaError::Failed(_) => {
            ApiError::bad_request("MEDIA_UNAVAILABLE", error.to_string())
        }
        other => ApiError::internal(other),
    })
}
