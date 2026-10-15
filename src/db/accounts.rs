use anyhow::Result;
use sqlx::SqliteExecutor;

use super::Db;
use crate::{debrid::AccountStatus, models::AccountRow, util::now};

pub async fn list(db: impl SqliteExecutor<'_>) -> Result<Vec<AccountRow>> {
    Ok(sqlx::query_as("SELECT * FROM accounts ORDER BY position, id").fetch_all(db).await?)
}

pub async fn get(db: &Db, id: i64) -> Result<Option<AccountRow>> {
    Ok(sqlx::query_as("SELECT * FROM accounts WHERE id = ?").bind(id).fetch_optional(db).await?)
}

/// Adds an account at the end of the list. Its status is unknown until the next check.
pub async fn insert(
    db: impl SqliteExecutor<'_>,
    provider: &str,
    label: &str,
    credentials: &str,
    enabled: bool,
) -> Result<i64> {
    Ok(sqlx::query_scalar(
        "INSERT INTO accounts (provider, label, credentials, enabled, position, created_at)
         VALUES (?, ?, ?, ?, (SELECT COALESCE(MAX(position), -1) + 1 FROM accounts), ?) RETURNING id",
    )
    .bind(provider)
    .bind(label)
    .bind(credentials)
    .bind(enabled)
    .bind(now())
    .fetch_one(db)
    .await?)
}

pub async fn update_settings(
    db: &Db,
    id: i64,
    label: Option<&str>,
    enabled: Option<bool>,
    credentials: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "UPDATE accounts SET label = COALESCE(?, label), enabled = COALESCE(?, enabled),
            credentials = COALESCE(?, credentials) WHERE id = ?",
    )
    .bind(label)
    .bind(enabled)
    .bind(credentials)
    .bind(id)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn update_credentials(db: &Db, id: i64, credentials: &str) -> Result<()> {
    sqlx::query("UPDATE accounts SET credentials = ? WHERE id = ?").bind(credentials).bind(id).execute(db).await?;
    Ok(())
}

pub async fn record_check(db: &Db, id: i64, result: Result<&AccountStatus, &str>) -> Result<()> {
    match result {
        Ok(status) => {
            sqlx::query(
                "UPDATE accounts SET username = ?, premium = ?, premium_until = ?, traffic_left = ?,
                    last_check = ?, last_error = NULL WHERE id = ?",
            )
            .bind(&status.username)
            .bind(status.premium)
            .bind(status.premium_until)
            .bind(status.traffic_left)
            .bind(now())
            .bind(id)
            .execute(db)
            .await?;
        }
        Err(error) => {
            sqlx::query("UPDATE accounts SET last_check = ?, last_error = ? WHERE id = ?")
                .bind(now())
                .bind(error)
                .bind(id)
                .execute(db)
                .await?;
        }
    }
    Ok(())
}

pub async fn delete(db: &Db, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM accounts WHERE id = ?").bind(id).execute(db).await?;
    Ok(())
}
