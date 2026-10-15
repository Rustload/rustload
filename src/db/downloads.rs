use std::collections::HashMap;

use anyhow::Result;
use sqlx::{QueryBuilder, Sqlite};

use super::{Db, packages::KnownLink};
use crate::{
    media::MediaMeta,
    models::{DownloadRow, DownloadStatus},
    util::{now, url_host},
};

pub struct NewDownload {
    pub url: String,
    pub rel_dir: String,
    pub filename: Option<String>,
    pub size: Option<i64>,
    pub host: Option<String>,
    pub account_id: Option<i64>,
    /// Set for links yt-dlp downloads although no media site rule knows them (HLS streams).
    pub media: Option<MediaMeta>,
}

impl NewDownload {
    /// A link known by its URL only.
    pub fn link(url: String) -> Self {
        Self {
            host: url_host(&url),
            url,
            rel_dir: String::new(),
            filename: None,
            size: None,
            account_id: None,
            media: None,
        }
    }
}

pub async fn insert_many(db: &Db, package_id: i64, items: &[NewDownload]) -> Result<()> {
    let mut tx = super::begin_write(db).await?;
    insert(&mut tx, package_id, 0, items).await?;
    tx.commit().await?;
    Ok(())
}

/// Replaces a playlist added as a single file by the links it lists, after the package's other
/// files. Does nothing if the file started meanwhile.
pub async fn replace_with_links(db: &Db, download_id: i64, package_id: i64, items: &[NewDownload]) -> Result<bool> {
    let mut tx = super::begin_write(db).await?;
    let removed = sqlx::query("DELETE FROM downloads WHERE id = ? AND status IN ('queued', 'paused')")
        .bind(download_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if removed == 0 {
        return Ok(false);
    }
    let first: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(position), -1) + 1 FROM downloads WHERE package_id = ?")
        .bind(package_id)
        .fetch_one(&mut *tx)
        .await?;
    insert(&mut tx, package_id, first, items).await?;
    tx.commit().await?;
    Ok(true)
}

async fn insert(tx: &mut sqlx::SqliteConnection, package_id: i64, first: i64, items: &[NewDownload]) -> Result<()> {
    let created = now();
    for (index, item) in items.iter().enumerate() {
        sqlx::query(
            "INSERT INTO downloads (package_id, url, rel_dir, filename, size, host, account_id, media, position,
                created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(package_id)
        .bind(&item.url)
        .bind(&item.rel_dir)
        .bind(&item.filename)
        .bind(item.size)
        .bind(&item.host)
        .bind(item.account_id)
        .bind(item.media.as_ref().map(serde_json::to_string).transpose()?)
        .bind(first + index as i64)
        .bind(created)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}

pub async fn get(db: &Db, id: i64) -> Result<Option<DownloadRow>> {
    Ok(sqlx::query_as("SELECT * FROM downloads WHERE id = ?").bind(id).fetch_optional(db).await?)
}

pub async fn list_by_package(db: &Db, package_id: i64) -> Result<Vec<DownloadRow>> {
    Ok(sqlx::query_as("SELECT * FROM downloads WHERE package_id = ? ORDER BY position, id")
        .bind(package_id)
        .fetch_all(db)
        .await?)
}

/// Files of several packages at once, grouped by package id.
pub async fn list_by_packages(db: &Db, package_ids: &[i64]) -> Result<HashMap<i64, Vec<DownloadRow>>> {
    let rows: Vec<DownloadRow> = sqlx::query_as(
        "SELECT * FROM downloads WHERE package_id IN (SELECT value FROM json_each(?)) ORDER BY package_id, position, id",
    )
    .bind(serde_json::to_string(package_ids)?)
    .fetch_all(db)
    .await?;
    let mut grouped: HashMap<i64, Vec<DownloadRow>> = HashMap::new();
    for row in rows {
        grouped.entry(row.package_id).or_default().push(row);
    }
    Ok(grouped)
}

/// Files ready to start, in queue order: queued, retry delay elapsed, package downloadable.
pub async fn next_candidates(db: &Db, limit: i64) -> Result<Vec<DownloadRow>> {
    Ok(sqlx::query_as(
        "SELECT d.* FROM downloads d JOIN packages p ON p.id = d.package_id
         WHERE d.status = 'queued' AND (d.retry_at IS NULL OR d.retry_at <= ?)
           AND p.stage = 'download' AND p.paused = 0 AND p.finished_at IS NULL
         ORDER BY p.position, p.id, d.position, d.id
         LIMIT ?",
    )
    .bind(now())
    .bind(limit)
    .fetch_all(db)
    .await?)
}

/// Queued files whose availability was not checked yet.
pub async fn list_unchecked(db: &Db, limit: i64) -> Result<Vec<DownloadRow>> {
    Ok(sqlx::query_as(
        "SELECT d.* FROM downloads d JOIN packages p ON p.id = d.package_id
         WHERE d.checked = 0 AND d.status IN ('queued', 'paused') AND p.finished_at IS NULL
         ORDER BY p.position, d.position LIMIT ?",
    )
    .bind(limit)
    .fetch_all(db)
    .await?)
}

/// Optional column updates, see [`super::packages::PackageUpdate`].
#[derive(Default)]
pub struct DownloadUpdate {
    pub status: Option<DownloadStatus>,
    pub error: Option<Option<String>>,
    pub filename: Option<Option<String>>,
    pub size: Option<Option<i64>>,
    pub downloaded: Option<i64>,
    pub via: Option<Option<String>>,
    pub host: Option<Option<String>>,
    pub path: Option<Option<String>>,
    pub temp_path: Option<Option<String>>,
    pub resume: Option<Option<String>>,
    pub checked: Option<bool>,
    pub attempts: Option<i64>,
    pub retry_at: Option<Option<i64>>,
    pub started_at: Option<Option<i64>>,
    pub finished_at: Option<Option<i64>>,
    pub expected_hash: Option<Option<String>>,
    pub checksum_algo: Option<Option<String>>,
    pub checksum_status: Option<Option<String>>,
    pub checksum_retries: Option<i64>,
}

pub async fn update(db: &Db, id: i64, update: DownloadUpdate) -> Result<()> {
    let mut query: QueryBuilder<Sqlite> = QueryBuilder::new("UPDATE downloads SET id = id");
    macro_rules! set {
        ($field:ident) => {
            if let Some(value) = update.$field {
                query.push(concat!(", ", stringify!($field), " = ")).push_bind(value);
            }
        };
    }
    set!(status);
    set!(error);
    set!(filename);
    set!(size);
    set!(downloaded);
    set!(via);
    set!(host);
    set!(path);
    set!(temp_path);
    set!(resume);
    set!(checked);
    set!(attempts);
    set!(retry_at);
    set!(started_at);
    set!(finished_at);
    set!(expected_hash);
    set!(checksum_algo);
    set!(checksum_status);
    set!(checksum_retries);
    query.push(" WHERE id = ").push_bind(id);
    query.build().execute(db).await?;
    Ok(())
}

/// Sets the status of every file of a package that currently has one of `from`.
pub async fn set_status_where(
    db: &Db,
    package_id: i64,
    from: &[DownloadStatus],
    to: DownloadStatus,
) -> Result<Vec<i64>> {
    let from: Vec<String> = from.iter().map(|s| status_name(*s)).collect();
    Ok(sqlx::query_scalar(
        "UPDATE downloads SET status = ?, retry_at = NULL
         WHERE package_id = ? AND status IN (SELECT value FROM json_each(?)) RETURNING id",
    )
    .bind(to)
    .bind(package_id)
    .bind(serde_json::to_string(&from)?)
    .fetch_all(db)
    .await?)
}

/// Re-queues failed files of a package and resets their retry counters.
pub async fn retry_failed(db: &Db, package_id: i64) -> Result<()> {
    sqlx::query(
        "UPDATE downloads SET status = 'queued', error = NULL, attempts = 0, retry_at = NULL
         WHERE package_id = ? AND status = 'failed'",
    )
    .bind(package_id)
    .execute(db)
    .await?;
    Ok(())
}

/// After a crash or restart nothing is running: put interrupted files back in the queue.
pub async fn requeue_interrupted(db: &Db) -> Result<u64> {
    Ok(sqlx::query("UPDATE downloads SET status = 'queued' WHERE status IN ('resolving', 'downloading')")
        .execute(db)
        .await?
        .rows_affected())
}

pub async fn delete(db: &Db, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM downloads WHERE id = ?").bind(id).execute(db).await?;
    Ok(())
}

pub async fn count_by_package(db: &Db, package_id: i64) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT COUNT(*) FROM downloads WHERE package_id = ?").bind(package_id).fetch_one(db).await?)
}

/// Remaining bytes and count of queued files (running ones are counted from live data).
pub async fn queue_remaining_bytes(db: &Db) -> Result<(i64, i64)> {
    let (remaining, queued): (Option<i64>, i64) = sqlx::query_as(
        "SELECT SUM(MAX(COALESCE(d.size, 0) - d.downloaded, 0)),
                COALESCE(SUM(CASE WHEN d.status = 'queued' THEN 1 ELSE 0 END), 0)
         FROM downloads d JOIN packages p ON p.id = d.package_id
         WHERE p.finished_at IS NULL AND p.paused = 0 AND d.status = 'queued'",
    )
    .fetch_one(db)
    .await?;
    Ok((remaining.unwrap_or(0), queued))
}

fn status_name(status: DownloadStatus) -> String {
    serde_json::to_value(status).ok().and_then(|v| v.as_str().map(str::to_owned)).unwrap_or_default()
}

/// Stores the result of an availability check without overwriting values set by a running task.
pub async fn record_check(
    db: &Db,
    id: i64,
    filename: Option<&str>,
    size: Option<i64>,
    host: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "UPDATE downloads SET filename = COALESCE(filename, ?), size = COALESCE(size, ?),
            host = COALESCE(host, ?), checked = 1 WHERE id = ?",
    )
    .bind(filename)
    .bind(size)
    .bind(host)
    .bind(id)
    .execute(db)
    .await?;
    Ok(())
}

/// Marks a not-yet-started file as offline. Returns `false` if it started (or went) meanwhile.
pub async fn mark_offline(db: &Db, id: i64, error: &str) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE downloads SET status = 'failed', error = ?, checked = 1 WHERE id = ? AND status IN ('queued', 'paused')",
    )
    .bind(error)
    .bind(id)
    .execute(db)
    .await?;
    Ok(result.rows_affected() == 1)
}

/// Packages holding a file with one of `urls` (exact match), unfinished packages first, then
/// newest first.
pub async fn find_by_urls(db: &Db, urls: &[&str]) -> Result<Vec<KnownLink>> {
    Ok(sqlx::query_as(
        "SELECT d.url AS link, p.id AS package_id, p.name AS package_name, p.finished_at IS NOT NULL AS finished
         FROM downloads d JOIN packages p ON p.id = d.package_id
         WHERE d.url IN (SELECT value FROM json_each(?))
         ORDER BY finished, p.id DESC",
    )
    .bind(serde_json::to_string(urls)?)
    .fetch_all(db)
    .await?)
}

/// Marks a completed file as counted in the statistics. Returns `false` if it already was: a file
/// downloaded again (checksum mismatch, retry) counts once.
pub async fn mark_counted(db: &Db, id: i64) -> Result<bool> {
    let result =
        sqlx::query("UPDATE downloads SET counted = 1 WHERE id = ? AND counted = 0").bind(id).execute(db).await?;
    Ok(result.rows_affected() == 1)
}

/// Marks a queued file as starting. Returns `false` if it is no longer queued (paused, removed or
/// already started), in which case the caller must not run it.
pub async fn claim(db: &Db, id: i64) -> Result<bool> {
    let result = sqlx::query(
        "UPDATE downloads SET status = 'resolving', error = NULL, retry_at = NULL,
            started_at = COALESCE(started_at, ?) WHERE id = ? AND status = 'queued'",
    )
    .bind(now())
    .bind(id)
    .execute(db)
    .await?;
    Ok(result.rows_affected() == 1)
}
