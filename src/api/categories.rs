//! Destination categories.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{AppState, Ids};
use crate::{
    db::{
        self, Table,
        categories::{CategoryInput, normalize_extensions},
    },
    engine::storage::category_path,
    error::{ApiError, ApiResult, OrNotFound},
    models::CategoryRow,
};

#[derive(Serialize)]
pub struct CategoryView {
    id: i64,
    name: String,
    path: String,
    resolved_path: String,
    extensions: Vec<String>,
    is_default: bool,
    position: i64,
}

fn view(state: &AppState, row: CategoryRow) -> CategoryView {
    CategoryView {
        resolved_path: category_path(&state.config.download_root, &row).to_string_lossy().into_owned(),
        extensions: row.extension_list(),
        id: row.id,
        name: row.name,
        path: row.path,
        is_default: row.is_default,
        position: row.position,
    }
}

pub async fn list(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let categories: Vec<CategoryView> =
        db::categories::list(state.db()).await?.into_iter().map(|row| view(&state, row)).collect();
    Ok(Json(json!({
        "categories": categories,
        "download_root": state.config.download_root.to_string_lossy(),
    })))
}

#[derive(Deserialize)]
pub struct CategoryBody {
    name: String,
    #[serde(default)]
    path: String,
    #[serde(default)]
    extensions: Vec<String>,
    #[serde(default)]
    is_default: bool,
}

impl CategoryBody {
    fn validate(self) -> ApiResult<CategoryInput> {
        let name = self.name.trim().to_owned();
        if name.is_empty() {
            return Err(ApiError::bad_request("INVALID_CATEGORY", "the category name is required"));
        }
        Ok(CategoryInput {
            name,
            path: self.path.trim().to_owned(),
            extensions: normalize_extensions(&self.extensions),
            is_default: self.is_default,
        })
    }
}

pub async fn create(State(state): State<AppState>, Json(body): Json<CategoryBody>) -> ApiResult<Json<CategoryView>> {
    let id = db::categories::insert(state.db(), &body.validate()?).await?;
    let row = db::categories::get(state.db(), id).await?.or_not_found("category")?;
    Ok(Json(view(&state, row)))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<CategoryBody>,
) -> ApiResult<Json<CategoryView>> {
    db::categories::get(state.db(), id).await?.or_not_found("category")?;
    db::categories::update(state.db(), id, &body.validate()?).await?;
    let row = db::categories::get(state.db(), id).await?.or_not_found("category")?;
    Ok(Json(view(&state, row)))
}

pub async fn remove(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    let category = db::categories::get(state.db(), id).await?.or_not_found("category")?;
    if category.is_default {
        return Err(ApiError::conflict("DEFAULT_CATEGORY", "the default category cannot be deleted"));
    }
    db::categories::delete(state.db(), id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn reorder(State(state): State<AppState>, Json(body): Json<Ids>) -> ApiResult<StatusCode> {
    db::reorder(state.db(), Table::Categories, &body.ids).await?;
    Ok(StatusCode::NO_CONTENT)
}
