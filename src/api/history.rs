//! Finished packages.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{
    AppState,
    packages::{DeleteQuery, delete_package, packages_response},
    views,
};
use crate::{
    db,
    engine::intake::{self, AddOptions, Source},
    error::{ApiResult, OrNotFound},
};

#[derive(Deserialize)]
pub struct HistoryQuery {
    #[serde(default)]
    q: String,
    #[serde(default = "default_limit")]
    limit: i64,
    #[serde(default)]
    offset: i64,
}

fn default_limit() -> i64 {
    50
}

pub async fn list(State(state): State<AppState>, Query(query): Query<HistoryQuery>) -> ApiResult<Json<Value>> {
    let limit = query.limit.clamp(1, 500);
    let (rows, total) = db::packages::list_history(state.db(), query.q.trim(), limit, query.offset.max(0)).await?;
    let items = views::package_views(&state.engine, state.db(), rows).await?;
    Ok(Json(json!({ "items": items, "total": total })))
}

pub async fn clear(State(state): State<AppState>) -> ApiResult<StatusCode> {
    db::packages::clear_history(state.db()).await?;
    state.engine.events.history_changed();
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<DeleteQuery>,
) -> ApiResult<StatusCode> {
    delete_package(&state, id, query, "history entry").await
}

/// Adds the same links again as a new package with the same options.
pub async fn redownload(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<Value>> {
    let package = db::packages::get(state.db(), id).await?.or_not_found("history entry")?;
    let source = Source::of(&state.engine, &package).await?;
    let id = intake::add_again(&state.engine, source, &AddOptions::from_package(&package)).await?;
    packages_response(&state, &[id]).await
}
