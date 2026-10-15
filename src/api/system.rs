//! Server information and global pause.

use axum::{Json, extract::State};
use serde_json::{Value, json};

use super::AppState;
use crate::error::ApiResult;

pub async fn info(State(state): State<AppState>) -> Json<Value> {
    let engine = &state.engine;
    Json(json!({
        "version": env!("CARGO_PKG_VERSION"),
        "download_root": state.config.download_root.to_string_lossy(),
        "uptime_secs": engine.started_at.elapsed().as_secs(),
        "extractor": engine.extractor.tool_name(),
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
