//! Download statistics (`stats_transfers`, `stats_days`) and the queue counts of `/metrics`.

use anyhow::Result;

use super::Db;

/// Aggregated completed files of one local day, hour, host and handler.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct TransferRow {
    pub day: String,
    pub hour: i64,
    pub host: String,
    pub via: String,
    pub bytes: i64,
    pub files: i64,
    pub secs: f64,
}

/// Daily counters.
#[derive(Debug, Clone, PartialEq, sqlx::FromRow)]
pub struct DayRow {
    pub day: String,
    pub packages: i64,
    pub failed: i64,
    pub peak_speed: Option<i64>,
}

/// Totals since statistics exist.
#[derive(Debug, Clone, Copy, Default, PartialEq, sqlx::FromRow)]
pub struct AllTimeRow {
    pub bytes: i64,
    pub files: i64,
    pub failed: i64,
    /// Completion time of the first recorded file.
    pub since: Option<i64>,
}

/// A completed file to count.
pub struct CompletedFile<'a> {
    pub day: &'a str,
    pub hour: u32,
    pub host: &'a str,
    pub via: &'a str,
    pub bytes: i64,
    pub secs: f64,
    pub at: i64,
}

pub async fn add_transfer(db: &Db, file: &CompletedFile<'_>) -> Result<()> {
    sqlx::query(
        "INSERT INTO stats_transfers (day, hour, host, via, bytes, files, secs, first_at)
         VALUES (?, ?, ?, ?, ?, 1, ?, ?)
         ON CONFLICT (day, hour, host, via) DO UPDATE SET
            bytes = bytes + excluded.bytes, files = files + 1, secs = secs + excluded.secs",
    )
    .bind(file.day)
    .bind(file.hour)
    .bind(file.host)
    .bind(file.via)
    .bind(file.bytes)
    .bind(file.secs)
    .bind(file.at)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn add_package(db: &Db, day: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO stats_days (day, packages) VALUES (?, 1)
         ON CONFLICT (day) DO UPDATE SET packages = packages + 1",
    )
    .bind(day)
    .execute(db)
    .await?;
    Ok(())
}

pub async fn add_failure(db: &Db, day: &str) -> Result<()> {
    sqlx::query(
        "INSERT INTO stats_days (day, failed) VALUES (?, 1)
         ON CONFLICT (day) DO UPDATE SET failed = failed + 1",
    )
    .bind(day)
    .execute(db)
    .await?;
    Ok(())
}

/// Stores `speed` as the peak of `day` unless a higher one is already stored.
pub async fn raise_peak(db: &Db, day: &str, speed: i64) -> Result<()> {
    sqlx::query(
        "INSERT INTO stats_days (day, peak_speed) VALUES (?, ?)
         ON CONFLICT (day) DO UPDATE SET peak_speed = MAX(COALESCE(peak_speed, 0), excluded.peak_speed)",
    )
    .bind(day)
    .bind(speed)
    .execute(db)
    .await?;
    Ok(())
}

/// Rows of the days `first..=last` (`YYYY-MM-DD` strings compare chronologically).
pub async fn transfers_between(db: &Db, first: &str, last: &str) -> Result<Vec<TransferRow>> {
    Ok(sqlx::query_as("SELECT day, hour, host, via, bytes, files, secs FROM stats_transfers WHERE day BETWEEN ? AND ?")
        .bind(first)
        .bind(last)
        .fetch_all(db)
        .await?)
}

pub async fn days_between(db: &Db, first: &str, last: &str) -> Result<Vec<DayRow>> {
    Ok(sqlx::query_as("SELECT day, packages, failed, peak_speed FROM stats_days WHERE day BETWEEN ? AND ?")
        .bind(first)
        .bind(last)
        .fetch_all(db)
        .await?)
}

pub async fn all_time(db: &Db) -> Result<AllTimeRow> {
    Ok(sqlx::query_as(
        "SELECT COALESCE(SUM(bytes), 0) AS bytes, COALESCE(SUM(files), 0) AS files,
                (SELECT COALESCE(SUM(failed), 0) FROM stats_days) AS failed, MIN(first_at) AS since
         FROM stats_transfers",
    )
    .fetch_one(db)
    .await?)
}

/// Number of files of unfinished packages per status (`queued`, `paused`, `failed`...).
pub async fn queue_status_counts(db: &Db) -> Result<Vec<(String, i64)>> {
    Ok(sqlx::query_as(
        "SELECT d.status, COUNT(*) FROM downloads d JOIN packages p ON p.id = d.package_id
         WHERE p.finished_at IS NULL GROUP BY d.status",
    )
    .fetch_all(db)
    .await?)
}
