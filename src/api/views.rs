//! JSON representations of packages and files (see `docs/API.md`).

use std::collections::HashMap;

use serde::Serialize;

use crate::{
    db::{self, Db},
    engine::{DebridProgress, Engine},
    media::{MediaMeta, MediaOptions},
    models::{DownloadRow, DownloadStatus, PackageRow, PackageStatus, SourceKind},
};

#[derive(Serialize)]
pub struct PackageView {
    id: i64,
    name: String,
    category_id: Option<i64>,
    dest_dir: Option<String>,
    create_subfolder: bool,
    extract: bool,
    delete_archives: bool,
    archive_password: Option<String>,
    source_kind: SourceKind,
    status: PackageStatus,
    paused: bool,
    error: Option<String>,
    debrid: Option<DebridProgress>,
    extract_progress: Option<f64>,
    verify_progress: Option<f64>,
    position: i64,
    created_at: i64,
    finished_at: Option<i64>,
    total_size: i64,
    downloaded: i64,
    media: Option<MediaOptions>,
    files: Vec<DownloadView>,
}

#[derive(Serialize)]
pub struct DownloadView {
    id: i64,
    package_id: i64,
    url: String,
    filename: Option<String>,
    size: Option<i64>,
    downloaded: i64,
    status: DownloadStatus,
    error: Option<String>,
    via: Option<String>,
    host: Option<String>,
    path: Option<String>,
    attempts: i64,
    retry_at: Option<i64>,
    position: i64,
    created_at: i64,
    started_at: Option<i64>,
    finished_at: Option<i64>,
    speed: u64,
    eta: Option<u64>,
    connections: u32,
    media: Option<MediaMeta>,
    map: Option<String>,
    heads: Vec<f64>,
    checksum: Option<ChecksumView>,
}

/// Result of the checksum verification of a file.
#[derive(Serialize)]
pub struct ChecksumView {
    algo: String,
    status: String,
}

/// Merges live transfer values into a stored file.
fn download_view(engine: &Engine, row: DownloadRow) -> DownloadView {
    let live = engine.live.get(row.id);
    let (map, heads) = live.as_ref().map(|live| live.segment_view()).unwrap_or_default();
    let (status, downloaded, size, speed, eta, connections) = match &live {
        Some(live) => (
            live.status(),
            live.downloaded.load(std::sync::atomic::Ordering::Relaxed) as i64,
            live.size().map(|s| s as i64).or(row.size),
            live.speed(),
            live.eta(),
            live.connections.load(std::sync::atomic::Ordering::Relaxed),
        ),
        None => (row.status, row.downloaded, row.size, 0, None, 0),
    };
    let media = row.media_meta();
    let checksum = match (row.checksum_algo, row.checksum_status) {
        (Some(algo), Some(status)) => Some(ChecksumView { algo, status }),
        _ => None,
    };
    DownloadView {
        id: row.id,
        package_id: row.package_id,
        url: row.url,
        filename: row.filename,
        size,
        downloaded,
        status,
        error: row.error,
        via: row.via,
        host: row.host,
        path: row.path,
        attempts: row.attempts,
        retry_at: row.retry_at,
        position: row.position,
        created_at: row.created_at,
        started_at: row.started_at,
        finished_at: row.finished_at,
        speed,
        eta,
        connections,
        map,
        heads,
        checksum,
        media,
    }
}

pub fn package_view(engine: &Engine, row: PackageRow, files: Vec<DownloadRow>) -> PackageView {
    let status = engine.package_status(&row, &files);
    let debrid = engine.remote_progress(row.id).or_else(|| row.remote_progress());
    let media = row.media_options();
    let files: Vec<DownloadView> = files.into_iter().map(|f| download_view(engine, f)).collect();
    PackageView {
        total_size: files.iter().filter_map(|f| f.size).sum(),
        downloaded: files.iter().map(|f| f.downloaded).sum(),
        id: row.id,
        name: row.name,
        category_id: row.category_id,
        dest_dir: row.dest_dir,
        create_subfolder: row.create_subfolder,
        extract: row.extract,
        delete_archives: row.delete_archives,
        archive_password: row.archive_password,
        source_kind: row.source_kind,
        status,
        paused: row.paused,
        error: row.error,
        debrid: if row.source_kind == SourceKind::Links { None } else { debrid },
        extract_progress: engine.live.extraction_progress(row.id),
        verify_progress: engine.live.verification_progress(row.id),
        position: row.position,
        created_at: row.created_at,
        finished_at: row.finished_at,
        media,
        files,
    }
}

/// Views for several packages, loading their files in one query.
pub async fn package_views(engine: &Engine, db: &Db, rows: Vec<PackageRow>) -> anyhow::Result<Vec<PackageView>> {
    let ids: Vec<i64> = rows.iter().map(|p| p.id).collect();
    let mut files: HashMap<i64, Vec<DownloadRow>> = db::downloads::list_by_packages(db, &ids).await?;
    Ok(rows
        .into_iter()
        .map(|row| {
            let package_files = files.remove(&row.id).unwrap_or_default();
            package_view(engine, row, package_files)
        })
        .collect())
}

pub async fn package_by_id(engine: &Engine, id: i64) -> anyhow::Result<Option<PackageView>> {
    let Some(row) = db::packages::get(&engine.db, id).await? else { return Ok(None) };
    let files = db::downloads::list_by_package(&engine.db, id).await?;
    Ok(Some(package_view(engine, row, files)))
}
