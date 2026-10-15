//! SQLite storage. Each submodule owns the queries of one table.

pub mod accounts;
pub mod categories;
pub mod downloads;
pub mod media;
pub mod packages;
pub mod stats;
pub mod users;

use std::{path::Path, time::Duration};

use anyhow::{Context, Result};
use sqlx::{
    AssertSqlSafe, SqliteConnection, SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions, SqliteSynchronous},
};

pub type Db = SqlitePool;

pub async fn open(path: &Path) -> Result<Db> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("cannot create {}", parent.display()))?;
    }
    let options = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .synchronous(SqliteSynchronous::Normal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(10));
    let pool = SqlitePoolOptions::new()
        .max_connections(8)
        .connect_with(options)
        .await
        .with_context(|| format!("cannot open database {}", path.display()))?;
    sqlx::migrate!("./migrations").run(&pool).await.context("database migration failed")?;
    Ok(pool)
}

pub async fn kv_get(db: &Db, key: &str) -> Result<Option<String>> {
    Ok(sqlx::query_scalar("SELECT value FROM kv WHERE key = ?").bind(key).fetch_optional(db).await?)
}

pub async fn kv_set(db: impl sqlx::SqliteExecutor<'_>, key: &str, value: &str) -> Result<()> {
    sqlx::query("INSERT INTO kv (key, value) VALUES (?, ?) ON CONFLICT(key) DO UPDATE SET value = excluded.value")
        .bind(key)
        .bind(value)
        .execute(db)
        .await?;
    Ok(())
}

/// Starts a transaction that writes. `BEGIN IMMEDIATE` takes the write lock at once, waiting for
/// it like any write; a plain (deferred) transaction that reads first cannot take it later while
/// another connection writes, and fails with "database is locked" instead of waiting.
pub async fn begin_write(db: &Db) -> Result<sqlx::SqliteTransaction<'static>> {
    Ok(db.begin_with("BEGIN IMMEDIATE").await?)
}

/// Checks that the database answers (health check).
pub async fn ping(db: &Db) -> Result<()> {
    sqlx::query("SELECT 1").execute(db).await?;
    Ok(())
}

/// Rewrites `position` of `table` rows so that they follow `ids`. Rows not listed keep their
/// relative order after the listed ones.
pub async fn reorder(db: &Db, table: Table, ids: &[i64]) -> Result<()> {
    let mut tx = begin_write(db).await?;
    reorder_in(&mut tx, table, ids).await?;
    tx.commit().await?;
    Ok(())
}

/// [`reorder`] inside a transaction.
pub async fn reorder_in(conn: &mut SqliteConnection, table: Table, ids: &[i64]) -> Result<()> {
    let sql = format!("UPDATE {} SET position = ? WHERE id = ?", table.name());
    for (index, id) in ids.iter().enumerate() {
        sqlx::query(AssertSqlSafe(sql.clone())).bind(index as i64).bind(id).execute(&mut *conn).await?;
    }
    let offset = ids.len() as i64;
    let sql = format!(
        "UPDATE {0} SET position = position + ? WHERE id NOT IN (SELECT value FROM json_each(?))",
        table.name()
    );
    sqlx::query(AssertSqlSafe(sql)).bind(offset).bind(serde_json::to_string(ids)?).execute(conn).await?;
    Ok(())
}

/// Tables with a user-controlled `position` column.
#[derive(Debug, Clone, Copy)]
pub enum Table {
    Packages,
    Categories,
    Accounts,
}

impl Table {
    fn name(self) -> &'static str {
        match self {
            Self::Packages => "packages",
            Self::Categories => "categories",
            Self::Accounts => "accounts",
        }
    }
}

/// Next free `position` value for a table.
pub async fn next_position(db: &Db, table: Table) -> Result<i64> {
    let sql = format!("SELECT COALESCE(MAX(position), -1) + 1 FROM {}", table.name());
    Ok(sqlx::query_scalar(AssertSqlSafe(sql)).fetch_one(db).await?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn write_transactions_wait_for_each_other() {
        let dir = tempfile::tempdir().unwrap();
        let db = open(&dir.path().join("rustload.db")).await.unwrap();
        let mut first = begin_write(&db).await.unwrap();
        kv_set(&mut *first, "value", "1").await.unwrap();
        // Reads then writes: with a deferred transaction, the write failed ("database is locked")
        // once the first transaction had committed in between.
        let second = tokio::spawn({
            let db = db.clone();
            async move {
                let mut tx = begin_write(&db).await?;
                let value: String =
                    sqlx::query_scalar("SELECT value FROM kv WHERE key = 'value'").fetch_one(&mut *tx).await?;
                kv_set(&mut *tx, "value", &format!("{value}2")).await?;
                tx.commit().await?;
                anyhow::Ok(())
            }
        });
        tokio::time::sleep(Duration::from_millis(200)).await;
        first.commit().await.unwrap();
        second.await.unwrap().unwrap();
        assert_eq!(kv_get(&db, "value").await.unwrap().as_deref(), Some("12"));
        db.close().await;
    }
}
