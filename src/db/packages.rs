use anyhow::Result;
use sqlx::{AssertSqlSafe, QueryBuilder, Sqlite};

use super::{Db, Table, next_position};
use crate::{
    models::{PackageRow, SourceKind, Stage},
    util::now,
};

pub struct NewPackage {
    pub name: String,
    pub name_locked: bool,
    pub category_id: Option<i64>,
    pub create_subfolder: bool,
    pub extract: bool,
    pub delete_archives: bool,
    pub archive_password: Option<String>,
    pub source_kind: SourceKind,
    pub source: Option<String>,
    pub stage: Stage,
    pub remote_account_id: Option<i64>,
    pub remote_id: Option<String>,
    pub paused: bool,
}

pub async fn insert(db: &Db, package: &NewPackage) -> Result<i64> {
    let position = next_position(db, Table::Packages).await?;
    let id = sqlx::query_scalar(
        "INSERT INTO packages (name, name_locked, category_id, create_subfolder, extract, delete_archives,
            archive_password, source_kind, source, stage, remote_account_id, remote_id, paused, position, created_at)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(&package.name)
    .bind(package.name_locked)
    .bind(package.category_id)
    .bind(package.create_subfolder)
    .bind(package.extract)
    .bind(package.delete_archives)
    .bind(&package.archive_password)
    .bind(package.source_kind)
    .bind(&package.source)
    .bind(package.stage)
    .bind(package.remote_account_id)
    .bind(&package.remote_id)
    .bind(package.paused)
    .bind(position)
    .bind(now())
    .fetch_one(db)
    .await?;
    Ok(id)
}

pub async fn get(db: &Db, id: i64) -> Result<Option<PackageRow>> {
    Ok(sqlx::query_as("SELECT * FROM packages WHERE id = ?").bind(id).fetch_optional(db).await?)
}

/// Unfinished packages, in queue order.
pub async fn list_queue(db: &Db) -> Result<Vec<PackageRow>> {
    Ok(sqlx::query_as("SELECT * FROM packages WHERE finished_at IS NULL ORDER BY position, id").fetch_all(db).await?)
}

/// Finished packages, newest first, optionally filtered by name or file name.
pub async fn list_history(db: &Db, query: &str, limit: i64, offset: i64) -> Result<(Vec<PackageRow>, i64)> {
    let pattern = format!("%{}%", query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_"));
    let filter = "finished_at IS NOT NULL AND (? = '%%' OR name LIKE ? ESCAPE '\\'
        OR EXISTS (SELECT 1 FROM downloads d WHERE d.package_id = packages.id AND d.filename LIKE ? ESCAPE '\\'))";
    let rows = sqlx::query_as(AssertSqlSafe(format!(
        "SELECT * FROM packages WHERE {filter} ORDER BY finished_at DESC, id DESC LIMIT ? OFFSET ?"
    )))
    .bind(&pattern)
    .bind(&pattern)
    .bind(&pattern)
    .bind(limit)
    .bind(offset)
    .fetch_all(db)
    .await?;
    let total = sqlx::query_scalar(AssertSqlSafe(format!("SELECT COUNT(*) FROM packages WHERE {filter}")))
        .bind(&pattern)
        .bind(&pattern)
        .bind(&pattern)
        .fetch_one(db)
        .await?;
    Ok((rows, total))
}

/// Packages whose magnet/torrent is still processed by a debrid service.
pub async fn list_remote(db: &Db) -> Result<Vec<PackageRow>> {
    Ok(sqlx::query_as(
        "SELECT * FROM packages WHERE stage = 'remote' AND error IS NULL AND finished_at IS NULL ORDER BY position",
    )
    .fetch_all(db)
    .await?)
}

pub async fn list_by_stage(db: &Db, stage: Stage) -> Result<Vec<PackageRow>> {
    Ok(sqlx::query_as("SELECT * FROM packages WHERE stage = ? AND finished_at IS NULL ORDER BY position")
        .bind(stage)
        .fetch_all(db)
        .await?)
}

/// Optional column updates; `None` leaves a column untouched. Nullable columns use
/// `Some(None)` to write NULL.
#[derive(Default)]
pub struct PackageUpdate {
    pub name: Option<String>,
    pub name_locked: Option<bool>,
    pub category_id: Option<Option<i64>>,
    pub dest_dir: Option<Option<String>>,
    pub create_subfolder: Option<bool>,
    pub extract: Option<bool>,
    pub delete_archives: Option<bool>,
    pub archive_password: Option<Option<String>>,
    pub stage: Option<Stage>,
    pub remote_account_id: Option<Option<i64>>,
    pub remote_id: Option<Option<String>>,
    pub remote_state: Option<Option<String>>,
    pub paused: Option<bool>,
    pub error: Option<Option<String>>,
    pub finished_at: Option<Option<i64>>,
}

pub async fn update(db: &Db, id: i64, update: PackageUpdate) -> Result<()> {
    let mut query: QueryBuilder<Sqlite> = QueryBuilder::new("UPDATE packages SET id = id");
    macro_rules! set {
        ($field:ident) => {
            if let Some(value) = update.$field {
                query.push(concat!(", ", stringify!($field), " = ")).push_bind(value);
            }
        };
    }
    set!(name);
    set!(name_locked);
    set!(category_id);
    set!(dest_dir);
    set!(create_subfolder);
    set!(extract);
    set!(delete_archives);
    set!(archive_password);
    set!(stage);
    set!(remote_account_id);
    set!(remote_id);
    set!(remote_state);
    set!(paused);
    set!(error);
    set!(finished_at);
    query.push(" WHERE id = ").push_bind(id);
    query.build().execute(db).await?;
    Ok(())
}

pub async fn delete(db: &Db, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM packages WHERE id = ?").bind(id).execute(db).await?;
    Ok(())
}

pub async fn clear_history(db: &Db) -> Result<()> {
    sqlx::query("DELETE FROM packages WHERE finished_at IS NOT NULL").execute(db).await?;
    Ok(())
}

pub async fn queue_ids(db: &Db) -> Result<Vec<i64>> {
    Ok(sqlx::query_scalar("SELECT id FROM packages WHERE finished_at IS NULL ORDER BY position, id")
        .fetch_all(db)
        .await?)
}

/// Moves a queued package to the given place.
pub async fn move_to(db: &Db, id: i64, to: MoveTarget) -> Result<()> {
    let mut ids = queue_ids(db).await?;
    let Some(index) = ids.iter().position(|&x| x == id) else { return Ok(()) };
    let item = ids.remove(index);
    let target = match to {
        MoveTarget::Top => 0,
        MoveTarget::Bottom => ids.len(),
        MoveTarget::Up => index.saturating_sub(1),
        MoveTarget::Down => (index + 1).min(ids.len()),
    };
    ids.insert(target, item);
    super::reorder(db, Table::Packages, &ids).await
}

#[derive(Debug, Clone, Copy, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MoveTarget {
    Top,
    Bottom,
    Up,
    Down,
}

/// Moves a package from one stage to another if it is still in `from`. Returns whether this call
/// made the transition (concurrent callers race for it).
pub async fn advance(db: &Db, id: i64, from: Stage, to: Stage) -> Result<bool> {
    let result = sqlx::query("UPDATE packages SET stage = ? WHERE id = ? AND stage = ?")
        .bind(to)
        .bind(id)
        .bind(from)
        .execute(db)
        .await?;
    Ok(result.rows_affected() == 1)
}

/// A link already held by a package: a file URL, or the URI of a magnet package.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct KnownLink {
    pub link: String,
    pub package_id: i64,
    pub package_name: String,
    /// The package is in the history.
    pub finished: bool,
}

/// URIs of magnet packages, unfinished packages first, then newest first.
pub async fn list_magnets(db: &Db) -> Result<Vec<KnownLink>> {
    Ok(sqlx::query_as(
        "SELECT source AS link, id AS package_id, name AS package_name, finished_at IS NOT NULL AS finished
         FROM packages WHERE source_kind = 'magnet' AND source IS NOT NULL
         ORDER BY finished, id DESC",
    )
    .fetch_all(db)
    .await?)
}

/// Marks a finished package as counted in the statistics. Returns `false` if it already was: a
/// package extracted again counts once.
pub async fn mark_counted(db: &Db, id: i64) -> Result<bool> {
    let result =
        sqlx::query("UPDATE packages SET counted = 1 WHERE id = ? AND counted = 0").bind(id).execute(db).await?;
    Ok(result.rows_affected() == 1)
}

/// Moves a package from `from` to the history; see [`advance`].
pub async fn complete(db: &Db, id: i64, from: Stage, error: Option<&str>) -> Result<bool> {
    let result =
        sqlx::query("UPDATE packages SET stage = 'done', error = ?, finished_at = ? WHERE id = ? AND stage = ?")
            .bind(error)
            .bind(now())
            .bind(id)
            .bind(from)
            .execute(db)
            .await?;
    Ok(result.rows_affected() == 1)
}
