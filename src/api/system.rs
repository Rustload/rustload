//! Server information and global pause.

use axum::{Json, extract::State};
use serde_json::{Value, json};

use super::{AppState, media::install_error};
use crate::{error::ApiResult, media::tools};

pub async fn info(State(state): State<AppState>) -> Json<Value> {
    let engine = &state.engine;
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "download_root": state.config.download_root.to_string_lossy(),
        "uptime_secs": engine.started_at.elapsed().as_secs(),
        "extractor": engine.extractor.tool_name(),
        // 7-Zip can be installed from the interface (Windows).
        "extractor_installable": tools::can_install_seven_zip(),
        "installing_extractor": engine.media.is_installing_seven_zip(),
        "paused": engine.is_paused(),
        "schedule_open": engine.schedule_open(),
        "disk": engine.disk_info(),
        "disk_low": engine.disk_low(),
        // Last detection, without running the tools again (see `/api/media/status`).
        "media": engine.media.cached().map(|tools| json!({
            "ytdlp": tools.ytdlp.and_then(|t| t.version),
            "ffmpeg": tools.ffmpeg.is_some(),
        })),
    }))
}

pub async fn pause(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    state.engine.set_queue_paused(true).await?;
    Ok(info(State(state)).await)
}

pub async fn resume(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    state.engine.set_queue_paused(false).await?;
    Ok(info(State(state)).await)
}

/// Installs 7-Zip into the config folder, on Windows (session only).
pub async fn install_seven_zip(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    match state.engine.media.install_seven_zip().await {
        Ok(version) => tracing::info!("{version} installed from the interface"),
        Err(error) => return Err(install_error("7-Zip", error)),
    }
    state.engine.extractor.refresh();
    Ok(info(State(state)).await)
}
