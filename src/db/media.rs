//! Media columns of `packages` and `downloads` (JSON documents, see `crate::media`).

use anyhow::Result;

use super::Db;
use crate::{
    media::{MediaMeta, MediaOptions},
    util::now,
};

/// A playlist entry to insert as a download. Entries come with what the playlist listing told
/// (title, duration), so they are marked as checked: probing hundreds of videos one by one would
/// only invite rate limiting.
pub struct NewEntry {
    pub url: String,
    pub filename: Option<String>,
    pub host: Option<String>,
    pub meta: MediaMeta,
}

pub async fn set_package_options(db: &Db, package_id: i64, options: Option<&MediaOptions>) -> Result<()> {
    let json = options.map(serde_json::to_string).transpose()?;
    sqlx::query("UPDATE packages SET media = ? WHERE id = ?").bind(json).bind(package_id).execute(db).await?;
    Ok(())
}

pub async fn set_download_meta(db: &Db, download_id: i64, meta: &MediaMeta) -> Result<()> {
    sqlx::query("UPDATE downloads SET media = ? WHERE id = ?")
        .bind(serde_json::to_string(meta)?)
        .bind(download_id)
        .execute(db)
        .await?;
    Ok(())
}

/// Appends entries to a package, after its existing files.
pub async fn insert_entries(db: &Db, package_id: i64, entries: &[NewEntry]) -> Result<()> {
    let mut tx = super::begin_write(db).await?;
    insert(&mut tx, package_id, entries).await?;
    tx.commit().await?;
    Ok(())
}

/// Replaces a playlist link that was added as a single file by its entries. Does nothing if the
/// file started meanwhile.
pub async fn replace_with_entries(db: &Db, download_id: i64, package_id: i64, entries: &[NewEntry]) -> Result<bool> {
    let mut tx = super::begin_write(db).await?;
    let removed = sqlx::query("DELETE FROM downloads WHERE id = ? AND status IN ('queued', 'paused')")
        .bind(download_id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if removed == 0 {
        return Ok(false);
    }
    insert(&mut tx, package_id, entries).await?;
    tx.commit().await?;
    Ok(true)
}

async fn insert(tx: &mut sqlx::SqliteConnection, package_id: i64, entries: &[NewEntry]) -> Result<()> {
    let first: i64 = sqlx::query_scalar("SELECT COALESCE(MAX(position), -1) + 1 FROM downloads WHERE package_id = ?")
        .bind(package_id)
        .fetch_one(&mut *tx)
        .await?;
    let created = now();
    for (index, entry) in entries.iter().enumerate() {
        sqlx::query(
            "INSERT INTO downloads (package_id, url, filename, host, media, checked, position, created_at)
             VALUES (?, ?, ?, ?, ?, 1, ?, ?)",
        )
        .bind(package_id)
        .bind(&entry.url)
        .bind(&entry.filename)
        .bind(&entry.host)
        .bind(serde_json::to_string(&entry.meta)?)
        .bind(first + index as i64)
        .bind(created)
        .execute(&mut *tx)
        .await?;
    }
    Ok(())
}
