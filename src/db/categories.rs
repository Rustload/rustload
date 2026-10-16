use std::collections::HashSet;

use anyhow::Result;
use sqlx::{SqliteConnection, SqliteExecutor};

use super::{Db, Table, next_position};
use crate::models::CategoryRow;

pub struct CategoryInput {
    pub name: String,
    pub path: String,
    pub extensions: Vec<String>,
    pub is_default: bool,
    /// Regular expression (see [`crate::engine::storage::rule`]), empty for none.
    pub pattern: String,
}

/// Extensions as stored: lowercase, without leading dot, letters and digits only, each once.
pub fn normalize_extensions(extensions: &[String]) -> Vec<String> {
    let mut seen = HashSet::new();
    extensions
        .iter()
        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|e| !e.is_empty() && e.chars().all(|c| c.is_ascii_alphanumeric()))
        .filter(|e| seen.insert(e.clone()))
        .collect()
}

pub async fn list(db: impl SqliteExecutor<'_>) -> Result<Vec<CategoryRow>> {
    Ok(sqlx::query_as("SELECT * FROM categories ORDER BY position, id").fetch_all(db).await?)
}

pub async fn get(db: &Db, id: i64) -> Result<Option<CategoryRow>> {
    Ok(sqlx::query_as("SELECT * FROM categories WHERE id = ?").bind(id).fetch_optional(db).await?)
}

pub async fn insert(db: &Db, input: &CategoryInput) -> Result<i64> {
    let position = next_position(db, Table::Categories).await?;
    let mut tx = super::begin_write(db).await?;
    let id = insert_at(&mut tx, input, position).await?;
    tx.commit().await?;
    Ok(id)
}

/// Inserts a category at `position`, inside a transaction.
pub async fn insert_at(conn: &mut SqliteConnection, input: &CategoryInput, position: i64) -> Result<i64> {
    if input.is_default {
        sqlx::query("UPDATE categories SET is_default = 0").execute(&mut *conn).await?;
    }
    Ok(sqlx::query_scalar(
        "INSERT INTO categories (name, path, extensions, pattern, is_default, position)
         VALUES (?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(&input.name)
    .bind(&input.path)
    .bind(input.extensions.join(","))
    .bind(&input.pattern)
    .bind(input.is_default)
    .bind(position)
    .fetch_one(conn)
    .await?)
}

pub async fn update(db: &Db, id: i64, input: &CategoryInput) -> Result<()> {
    let mut tx = super::begin_write(db).await?;
    update_in(&mut tx, id, input).await?;
    tx.commit().await?;
    Ok(())
}

/// Updates a category inside a transaction.
pub async fn update_in(conn: &mut SqliteConnection, id: i64, input: &CategoryInput) -> Result<()> {
    if input.is_default {
        sqlx::query("UPDATE categories SET is_default = 0").execute(&mut *conn).await?;
    }
    // The default flag can be moved to another category but never removed from the last holder.
    sqlx::query(
        "UPDATE categories SET name = ?, path = ?, extensions = ?, pattern = ?,
            is_default = CASE WHEN ? THEN 1 ELSE is_default END
         WHERE id = ?",
    )
    .bind(&input.name)
    .bind(&input.path)
    .bind(input.extensions.join(","))
    .bind(&input.pattern)
    .bind(input.is_default)
    .bind(id)
    .execute(conn)
    .await?;
    Ok(())
}

/// Makes `id` the one default category.
pub async fn set_default(db: impl SqliteExecutor<'_>, id: i64) -> Result<()> {
    sqlx::query("UPDATE categories SET is_default = (id = ?)").bind(id).execute(db).await?;
    Ok(())
}

pub async fn delete(db: &Db, id: i64) -> Result<()> {
    sqlx::query("DELETE FROM categories WHERE id = ? AND is_default = 0").bind(id).execute(db).await?;
    Ok(())
}

/// Creates the one category of a new installation: the download root, used by default. Users add
/// their own folders from there.
pub async fn seed_defaults(db: &Db) -> Result<()> {
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM categories").fetch_one(db).await?;
    if count > 0 {
        return Ok(());
    }
    let downloads = CategoryInput {
        name: "Downloads".into(),
        path: String::new(),
        extensions: Vec::new(),
        is_default: true,
        pattern: String::new(),
    };
    insert(db, &downloads).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_extensions() {
        let raw: Vec<String> = [".MKV", " mp4 ", "mkv", "", "bad ext", "m2ts", "Mp4"].map(str::to_owned).into();
        assert_eq!(normalize_extensions(&raw), ["mkv", "mp4", "m2ts"]);
    }
}
