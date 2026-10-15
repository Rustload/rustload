//! Queue endpoints: packages and their files.

use axum::{
    Json,
    extract::{Multipart, Path, Query, State},
    http::StatusCode,
};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{AppState, Ids, views};
use crate::{
    db::{
        self, Table,
        packages::{MoveTarget, PackageUpdate},
    },
    engine::{
        intake::{self, AddOptions, IntakeError},
        media::package_options,
    },
    error::{ApiError, ApiResult, OrNotFound},
    media::MediaOptionsPatch,
};

impl From<IntakeError> for ApiError {
    fn from(error: IntakeError) -> Self {
        match error {
            IntakeError::NoLinks => ApiError::bad_request("NO_LINKS", "no link found"),
            IntakeError::NoMagnetAccount => ApiError::bad_request("NO_MAGNET_ACCOUNT", error.to_string()),
            IntakeError::InvalidMediaOptions(_) => ApiError::bad_request("INVALID_MEDIA_OPTIONS", error.to_string()),
            IntakeError::TorrentGone(_) => ApiError::bad_request("TORRENT_GONE", error.to_string()),
            IntakeError::Debrid(error) => ApiError::bad_request("DEBRID_ERROR", error.to_string()),
            IntakeError::Internal(error) => ApiError::internal(error),
        }
    }
}

pub async fn list(State(state): State<AppState>) -> ApiResult<Json<Value>> {
    let rows = db::packages::list_queue(state.db()).await?;
    let packages = views::package_views(&state.engine, state.db(), rows).await?;
    Ok(Json(json!({ "packages": packages })))
}

#[derive(Deserialize)]
pub struct AddRequest {
    links: String,
    #[serde(flatten)]
    options: AddOptions,
}

pub async fn add(State(state): State<AppState>, Json(body): Json<AddRequest>) -> ApiResult<Json<Value>> {
    validate_add(&state, &body, "").await?;
    let ids = intake::add_text(&state.engine, &body.links, &body.options).await?;
    packages_response(&state, &ids).await
}

/// Packages added by one batch request at most.
const MAX_BATCH: usize = 100;

#[derive(Deserialize)]
pub struct BatchRequest {
    packages: Vec<AddRequest>,
}

/// Adds several packages at once (link grabber, browser extension). Every request is validated
/// before the first package is created.
pub async fn add_batch(State(state): State<AppState>, Json(body): Json<BatchRequest>) -> ApiResult<Json<Value>> {
    if body.packages.is_empty() || body.packages.len() > MAX_BATCH {
        return Err(ApiError::bad_request("INVALID_BATCH", format!("send between 1 and {MAX_BATCH} packages")));
    }
    for (index, request) in body.packages.iter().enumerate() {
        validate_add(&state, request, &format!("package {}: ", index + 1)).await?;
    }
    let mut ids = Vec::new();
    for request in &body.packages {
        ids.extend(intake::add_text(&state.engine, &request.links, &request.options).await?);
    }
    packages_response(&state, &ids).await
}

/// Refuses what intake would reject half-way: no link, an unknown category, magnets without an
/// account able to process them. `context` prefixes error messages.
async fn validate_add(state: &AppState, request: &AddRequest, context: &str) -> ApiResult<()> {
    let links = intake::parse_links(&request.links);
    if links.http.is_empty() && links.magnets.is_empty() {
        return Err(ApiError::bad_request("NO_LINKS", format!("{context}no link found")));
    }
    if let Some(id) = request.options.category_id
        && db::categories::get(state.db(), id).await?.is_none()
    {
        return Err(ApiError::bad_request("UNKNOWN_CATEGORY", format!("{context}category {id} does not exist")));
    }
    if !links.magnets.is_empty() && state.engine.debrid.magnet_account().await.is_none() {
        let message = format!("{context}{}", IntakeError::NoMagnetAccount);
        return Err(ApiError::bad_request("NO_MAGNET_ACCOUNT", message));
    }
    Ok(())
}

/// Multipart: one or more `file` parts (.torrent) and an optional `options` part (JSON).
pub async fn add_torrent(State(state): State<AppState>, mut multipart: Multipart) -> ApiResult<Json<Value>> {
    let mut options = AddOptions::default();
    let mut torrents = Vec::new();
    while let Some(field) =
        multipart.next_field().await.map_err(|e| ApiError::bad_request("BAD_UPLOAD", e.to_string()))?
    {
        match field.name() {
            Some("options") => {
                let text = field.text().await.map_err(|e| ApiError::bad_request("BAD_UPLOAD", e.to_string()))?;
                options =
                    serde_json::from_str(&text).map_err(|e| ApiError::bad_request("BAD_OPTIONS", e.to_string()))?;
            }
            Some("file") => {
                let name = field.file_name().unwrap_or("upload.torrent").to_owned();
                let data = field.bytes().await.map_err(|e| ApiError::bad_request("BAD_UPLOAD", e.to_string()))?;
                torrents.push((name, data.to_vec()));
            }
            _ => {}
        }
    }
    if torrents.is_empty() {
        return Err(ApiError::bad_request("NO_FILE", "no .torrent file received"));
    }
    let mut ids = Vec::new();
    for (name, data) in torrents {
        ids.push(intake::add_torrent(&state.engine, &name, data, &options).await?);
    }
    packages_response(&state, &ids).await
}

pub(super) async fn packages_response(state: &AppState, ids: &[i64]) -> ApiResult<Json<Value>> {
    let mut packages = Vec::new();
    for id in ids {
        if let Some(view) = views::package_by_id(&state.engine, *id).await? {
            packages.push(view);
        }
    }
    Ok(Json(json!({ "packages": packages })))
}

async fn package_response(state: &AppState, id: i64) -> ApiResult<Json<views::PackageView>> {
    Ok(Json(views::package_by_id(&state.engine, id).await?.or_not_found("package")?))
}

/// `Some(None)` for an explicit `null`, `None` when the field is absent.
fn nullable<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

#[derive(Deserialize)]
pub struct PackagePatch {
    name: Option<String>,
    #[serde(default, deserialize_with = "nullable")]
    category_id: Option<Option<i64>>,
    create_subfolder: Option<bool>,
    extract: Option<bool>,
    delete_archives: Option<bool>,
    #[serde(default, deserialize_with = "nullable")]
    archive_password: Option<Option<String>>,
    /// Applies to files that have not started yet; missing fields keep their current value.
    media: Option<MediaOptionsPatch>,
}

pub async fn update(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(patch): Json<PackagePatch>,
) -> ApiResult<Json<views::PackageView>> {
    let package = db::packages::get(state.db(), id).await?.or_not_found("package")?;
    let changes_destination = patch.category_id.is_some_and(|c| c != package.category_id)
        || patch.create_subfolder.is_some_and(|s| s != package.create_subfolder);
    if changes_destination && package.dest_dir.is_some() {
        return Err(ApiError::conflict("DEST_LOCKED", "the destination cannot change once a file has started"));
    }
    if let Some(media) = &patch.media {
        let current = package_options(&state.engine.settings.get(), &package);
        let options = media
            .apply(&current)
            .validate()
            .map_err(|e| ApiError::bad_request("INVALID_MEDIA_OPTIONS", format!("invalid media options: {e}")))?;
        db::media::set_package_options(state.db(), id, Some(&options)).await?;
    }
    let name = patch.name.map(|n| n.trim().to_owned()).filter(|n| !n.is_empty());
    let update = PackageUpdate {
        name_locked: name.as_ref().map(|_| true),
        name,
        category_id: patch.category_id,
        create_subfolder: patch.create_subfolder,
        extract: patch.extract,
        delete_archives: patch.delete_archives,
        archive_password: patch.archive_password.map(|p| p.filter(|p| !p.trim().is_empty())),
        ..Default::default()
    };
    db::packages::update(state.db(), id, update).await?;
    state.engine.events.queue_changed();
    state.engine.events.history_changed();
    package_response(&state, id).await
}

/// `?files=true` also deletes the downloaded files.
#[derive(Deserialize)]
pub struct DeleteQuery {
    #[serde(default)]
    files: bool,
}

pub async fn remove(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Query(query): Query<DeleteQuery>,
) -> ApiResult<StatusCode> {
    delete_package(&state, id, query, "package").await
}

/// Deletes a queued or finished package; `what` names it in the "not found" error.
pub(super) async fn delete_package(state: &AppState, id: i64, query: DeleteQuery, what: &str) -> ApiResult<StatusCode> {
    db::packages::get(state.db(), id).await?.or_not_found(what)?;
    state.engine.delete_package(id, query.files).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn pause(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<views::PackageView>> {
    state.engine.pause_package(id).await?;
    package_response(&state, id).await
}

pub async fn resume(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<views::PackageView>> {
    state.engine.resume_package(id).await?;
    package_response(&state, id).await
}

pub async fn retry(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<views::PackageView>> {
    state.engine.retry_package(id).await.map_err(|e| ApiError::bad_request("RETRY_FAILED", format!("{e:#}")))?;
    package_response(&state, id).await
}

pub async fn extract(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<Json<views::PackageView>> {
    db::packages::get(state.db(), id).await?.or_not_found("package")?;
    if !state.engine.extract_package(id).await? {
        return Err(ApiError::conflict("EXTRACT_NOT_READY", "the package still has files to download"));
    }
    package_response(&state, id).await
}

#[derive(Deserialize)]
pub struct MoveRequest {
    to: MoveTarget,
}

pub async fn move_to(
    State(state): State<AppState>,
    Path(id): Path<i64>,
    Json(body): Json<MoveRequest>,
) -> ApiResult<StatusCode> {
    db::packages::move_to(state.db(), id, body.to).await?;
    state.engine.events.queue_changed();
    state.engine.wake();
    Ok(StatusCode::NO_CONTENT)
}

pub async fn reorder(State(state): State<AppState>, Json(body): Json<Ids>) -> ApiResult<StatusCode> {
    db::reorder(state.db(), Table::Packages, &body.ids).await?;
    state.engine.events.queue_changed();
    state.engine.wake();
    Ok(StatusCode::NO_CONTENT)
}

pub async fn pause_download(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    state.engine.pause_download(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn resume_download(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    state.engine.resume_download(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn retry_download(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    db::downloads::get(state.db(), id).await?.or_not_found("download")?;
    state.engine.retry_download(id).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn remove_download(State(state): State<AppState>, Path(id): Path<i64>) -> ApiResult<StatusCode> {
    db::downloads::get(state.db(), id).await?.or_not_found("download")?;
    state.engine.delete_download(id).await?;
    Ok(StatusCode::NO_CONTENT)
}
