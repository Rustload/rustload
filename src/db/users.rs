use anyhow::Result;

use super::Db;
use crate::util::now;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserRow {
    pub id: i64,
    pub username: String,
    pub password_hash: String,
}

pub async fn count(db: &Db) -> Result<i64> {
    Ok(sqlx::query_scalar("SELECT COUNT(*) FROM users").fetch_one(db).await?)
}

pub async fn find_by_name(db: &Db, username: &str) -> Result<Option<UserRow>> {
    Ok(sqlx::query_as("SELECT id, username, password_hash FROM users WHERE username = ?")
        .bind(username)
        .fetch_optional(db)
        .await?)
}

pub async fn get(db: &Db, id: i64) -> Result<Option<UserRow>> {
    Ok(sqlx::query_as("SELECT id, username, password_hash FROM users WHERE id = ?").bind(id).fetch_optional(db).await?)
}

/// The administrator created first (the desktop app's only user).
pub async fn first(db: &Db) -> Result<Option<UserRow>> {
    Ok(sqlx::query_as("SELECT id, username, password_hash FROM users ORDER BY id LIMIT 1").fetch_optional(db).await?)
}

pub async fn insert(db: &Db, username: &str, password_hash: &str) -> Result<i64> {
    Ok(sqlx::query_scalar("INSERT INTO users (username, password_hash, created_at) VALUES (?, ?, ?) RETURNING id")
        .bind(username)
        .bind(password_hash)
        .bind(now())
        .fetch_one(db)
        .await?)
}

/// Creates the first user, unless one already exists. Returns `None` if one did.
pub async fn insert_first(db: &Db, username: &str, password_hash: &str) -> Result<Option<i64>> {
    Ok(sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, created_at)
         SELECT ?, ?, ? WHERE NOT EXISTS (SELECT 1 FROM users) RETURNING id",
    )
    .bind(username)
    .bind(password_hash)
    .bind(now())
    .fetch_optional(db)
    .await?)
}

pub async fn set_password(db: &Db, id: i64, password_hash: &str) -> Result<()> {
    sqlx::query("UPDATE users SET password_hash = ? WHERE id = ?").bind(password_hash).bind(id).execute(db).await?;
    Ok(())
}

pub async fn insert_session(db: &Db, token_hash: &str, user_id: i64, expires_at: i64) -> Result<()> {
    sqlx::query("INSERT INTO sessions (token_hash, user_id, created_at, expires_at) VALUES (?, ?, ?, ?)")
        .bind(token_hash)
        .bind(user_id)
        .bind(now())
        .bind(expires_at)
        .execute(db)
        .await?;
    Ok(())
}

/// User owning a valid (non expired) session.
pub async fn session_user(db: &Db, token_hash: &str) -> Result<Option<UserRow>> {
    Ok(sqlx::query_as(
        "SELECT u.id, u.username, u.password_hash FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = ? AND s.expires_at > ?",
    )
    .bind(token_hash)
    .bind(now())
    .fetch_optional(db)
    .await?)
}

/// Moves the expiry of a valid session to `expires_at` if it expires before `threshold`. Returns
/// whether it did: most calls write nothing.
pub async fn extend_session(db: &Db, token_hash: &str, expires_at: i64, threshold: i64) -> Result<bool> {
    let updated =
        sqlx::query("UPDATE sessions SET expires_at = ? WHERE token_hash = ? AND expires_at < ? AND expires_at > ?")
            .bind(expires_at)
            .bind(token_hash)
            .bind(threshold)
            .bind(now())
            .execute(db)
            .await?;
    Ok(updated.rows_affected() > 0)
}

pub async fn delete_session(db: &Db, token_hash: &str) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?").bind(token_hash).execute(db).await?;
    Ok(())
}

/// Revokes every session of a user except the given one (after a password change).
pub async fn delete_other_sessions(db: &Db, user_id: i64, keep_token_hash: &str) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE user_id = ? AND token_hash != ?")
        .bind(user_id)
        .bind(keep_token_hash)
        .execute(db)
        .await?;
    Ok(())
}

pub async fn purge_expired_sessions(db: &Db) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE expires_at <= ?").bind(now()).execute(db).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn extends_only_sessions_close_to_their_end() {
        let dir = tempfile::tempdir().unwrap();
        let db = crate::db::open(&dir.path().join("rustload.db")).await.unwrap();
        let user = insert(&db, "me", "hash").await.unwrap();
        let now = now();
        insert_session(&db, "fresh", user, now + 1000).await.unwrap();
        insert_session(&db, "old", user, now + 10).await.unwrap();
        insert_session(&db, "expired", user, now - 10).await.unwrap();

        assert!(!extend_session(&db, "fresh", now + 2000, now + 500).await.unwrap());
        assert!(extend_session(&db, "old", now + 2000, now + 500).await.unwrap());
        assert!(!extend_session(&db, "old", now + 2000, now + 500).await.unwrap());
        assert!(!extend_session(&db, "expired", now + 2000, now + 500).await.unwrap());
        assert!(session_user(&db, "old").await.unwrap().is_some());
        assert!(session_user(&db, "expired").await.unwrap().is_none());
    }
}
