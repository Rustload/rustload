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
    engine::storage::{category_path, missing_mount, rule},
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
    pattern: String,
    /// The folder is outside the download root and does not exist (see [`missing_mount`]).
    missing: bool,
}

async fn view(state: &AppState, row: CategoryRow) -> CategoryView {
    let root = &state.config.download_root;
    CategoryView {
        resolved_path: category_path(root, &row).to_string_lossy().into_owned(),
        missing: missing_mount(root, &row).await.is_some(),
        extensions: row.extension_list(),
        id: row.id,
        name: row.name,
        path: row.path,
        is_default: row.is_default,
        position: row.position,
        pattern: row.pattern,
    }
}

pub async fn list(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let mut categories = Vec::new();
    for row in db::categories::list(state.db()).await? {
        categories.push(view(&state, row).await);
    }
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
    #[serde(default)]
    pattern: String,
}

/// Longest rule accepted.
const MAX_PATTERN_LEN: usize = 500;

/// The reason only: the full message of a syntax error draws the rule with a caret over lines.
fn rule_error(error: &regex::Error) -> String {
    match error {
        regex::Error::Syntax(message) => {
            message.lines().last().unwrap_or_default().trim().trim_start_matches("error: ").to_owned()
        }
        _ => "it is too complex".into(),
    }
}

impl CategoryBody {
    fn validate(self) -> ApiResult<CategoryInput> {
        let name = self.name.trim().to_owned();
        if name.is_empty() {
            return Err(ApiError::bad_request("INVALID_CATEGORY", "the category name is required"));
        }
        let pattern = self.pattern.trim().to_owned();
        if pattern.chars().count() > MAX_PATTERN_LEN {
            return Err(ApiError::bad_request("INVALID_PATTERN", "the rule is too long"));
        }
        if !pattern.is_empty()
            && let Err(error) = rule(&pattern)
        {
            return Err(ApiError::bad_request("INVALID_PATTERN", format!("invalid rule: {}", rule_error(&error))));
        }
        Ok(CategoryInput {
            name,
            path: self.path.trim().to_owned(),
            extensions: normalize_extensions(&self.extensions),
            is_default: self.is_default,
            pattern,
        })
    }
}

pub async fn create(State(state): State<AppState>, Json(body): Json<CategoryBody>) -> ApiResult<Json<CategoryView>> {
    let id = db::categories::insert(state.db(), &body.validate()?).await?;
    let row = db::categories::get(state.db(), id).await?.or_not_found("category")?;
    Ok(Json(view(&state, row).await))
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<CategoryBody>,
) -> ApiResult<Json<CategoryView>> {
    db::categories::get(state.db(), id).await?.or_not_found("category")?;
    db::categories::update(state.db(), id, &body.validate()?).await?;
    let row = db::categories::get(state.db(), id).await?.or_not_found("category")?;
    Ok(Json(view(&state, row).await))
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
