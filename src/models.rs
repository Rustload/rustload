//! Database rows and the enums stored in them.

use std::path::PathBuf;

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use sqlx::FromRow;

use crate::{
    engine::DebridProgress,
    media::{MediaMeta, MediaOptions},
};

/// Lifecycle of a single file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(rename_all = "lowercase")]
pub enum DownloadStatus {
    Queued,
    Resolving,
    Downloading,
    Paused,
    Completed,
    Failed,
    Skipped,
}

impl DownloadStatus {
    /// Completed or skipped: nothing left to do for this file.
    pub fn is_done(self) -> bool {
        matches!(self, Self::Completed | Self::Skipped)
    }

    pub fn is_active(self) -> bool {
        matches!(self, Self::Resolving | Self::Downloading)
    }
}

/// Where a package currently is in its pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(rename_all = "lowercase")]
pub enum Stage {
    /// A magnet/torrent is being downloaded by the debrid service.
    Remote,
    Download,
    Extract,
    Done,
    /// Files are checked against checksums before extraction.
    Verify,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "lowercase")]
#[sqlx(rename_all = "lowercase")]
pub enum SourceKind {
    Links,
    Magnet,
    Torrent,
}

/// Status shown to users, derived from the package row and its files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageStatus {
    Queued,
    Downloading,
    Paused,
    Debrid,
    Extracting,
    Completed,
    Failed,
    Verifying,
}

#[derive(Debug, Clone, FromRow)]
pub struct PackageRow {
    pub id: i64,
    pub name: String,
    pub category_id: Option<i64>,
    pub dest_dir: Option<String>,
    pub create_subfolder: bool,
    pub extract: bool,
    pub delete_archives: bool,
    pub archive_password: Option<String>,
    pub source_kind: SourceKind,
    pub source: Option<String>,
    pub stage: Stage,
    pub remote_account_id: Option<i64>,
    pub remote_id: Option<String>,
    pub remote_state: Option<String>,
    pub name_locked: bool,
    pub paused: bool,
    pub error: Option<String>,
    pub position: i64,
    pub created_at: i64,
    pub finished_at: Option<i64>,
    /// JSON `MediaOptions`, for packages with media links.
    pub media: Option<String>,
}

impl PackageRow {
    pub fn status(&self, files: &[DownloadRow]) -> PackageStatus {
        if self.finished_at.is_some() || self.stage == Stage::Done {
            return if self.error.is_some() { PackageStatus::Failed } else { PackageStatus::Completed };
        }
        match self.stage {
            Stage::Remote if self.error.is_some() => PackageStatus::Failed,
            Stage::Remote => PackageStatus::Debrid,
            Stage::Extract => PackageStatus::Extracting,
            Stage::Verify => PackageStatus::Verifying,
            Stage::Done => unreachable!("handled above"),
            Stage::Download => {
                let any = |s: DownloadStatus| files.iter().any(|f| f.status == s);
                if files.iter().any(|f| f.status.is_active()) {
                    PackageStatus::Downloading
                } else if self.paused {
                    PackageStatus::Paused
                } else if any(DownloadStatus::Failed) && !any(DownloadStatus::Queued) {
                    PackageStatus::Failed
                } else if !files.is_empty() && files.iter().all(|f| f.status.is_done()) {
                    PackageStatus::Completed
                } else if !any(DownloadStatus::Queued) && any(DownloadStatus::Paused) {
                    PackageStatus::Paused
                } else {
                    PackageStatus::Queued
                }
            }
        }
    }

    pub fn media_options(&self) -> Option<MediaOptions> {
        from_json(self.media.as_deref())
    }

    /// Progress of the magnet/torrent at the debrid service, as last saved.
    pub fn remote_progress(&self) -> Option<DebridProgress> {
        from_json(self.remote_state.as_deref())
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct DownloadRow {
    pub id: i64,
    pub package_id: i64,
    pub url: String,
    pub rel_dir: String,
    pub filename: Option<String>,
    pub size: Option<i64>,
    pub downloaded: i64,
    pub status: DownloadStatus,
    pub error: Option<String>,
    pub via: Option<String>,
    pub host: Option<String>,
    pub account_id: Option<i64>,
    pub path: Option<String>,
    pub temp_path: Option<String>,
    pub resume: Option<String>,
    pub attempts: i64,
    pub retry_at: Option<i64>,
    pub position: i64,
    pub created_at: i64,
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
    /// `<algo>:<hex>` reported by the provider, see [`crate::checksum::FileHash`].
    pub expected_hash: Option<String>,
    pub checksum_algo: Option<String>,
    /// `ok` or `mismatch`.
    pub checksum_status: Option<String>,
    pub checksum_retries: i64,
    /// JSON `MediaMeta` of a media link (title, duration...).
    pub media: Option<String>,
}

impl DownloadRow {
    pub fn media_meta(&self) -> Option<MediaMeta> {
        from_json(self.media.as_deref())
    }

    /// Where the file is, once downloaded.
    pub fn completed_path(&self) -> Option<PathBuf> {
        self.path.as_deref().filter(|_| self.status == DownloadStatus::Completed).map(PathBuf::from)
    }
}

/// Reads a JSON column; unreadable values count as absent.
fn from_json<T: DeserializeOwned>(json: Option<&str>) -> Option<T> {
    serde_json::from_str(json?).ok()
}

#[derive(Debug, Clone, FromRow)]
pub struct CategoryRow {
    pub id: i64,
    pub name: String,
    pub path: String,
    pub extensions: String,
    pub is_default: bool,
    pub position: i64,
}

impl CategoryRow {
    pub fn extension_list(&self) -> Vec<String> {
        self.extensions.split(',').map(str::trim).filter(|e| !e.is_empty()).map(str::to_owned).collect()
    }
}

#[derive(Debug, Clone, FromRow)]
pub struct AccountRow {
    pub id: i64,
    pub provider: String,
    pub label: String,
    pub credentials: String,
    pub enabled: bool,
    pub position: i64,
    pub username: Option<String>,
    pub premium: bool,
    pub premium_until: Option<i64>,
    pub traffic_left: Option<i64>,
    pub last_check: Option<i64>,
    pub last_error: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn package(stage: Stage) -> PackageRow {
        PackageRow {
            id: 1,
            name: "p".into(),
            category_id: None,
            dest_dir: None,
            create_subfolder: true,
            extract: false,
            delete_archives: false,
            archive_password: None,
            source_kind: SourceKind::Links,
            source: None,
            stage,
            remote_account_id: None,
            remote_id: None,
            remote_state: None,
            name_locked: false,
            paused: false,
            error: None,
            position: 0,
            created_at: 0,
            finished_at: None,
            media: None,
        }
    }

    fn file(status: DownloadStatus) -> DownloadRow {
        DownloadRow {
            id: 1,
            package_id: 1,
            url: "https://example.com/a".into(),
            rel_dir: String::new(),
            filename: None,
            size: None,
            downloaded: 0,
            status,
            error: None,
            via: None,
            host: None,
            account_id: None,
            path: None,
            temp_path: None,
            resume: None,
            attempts: 0,
            retry_at: None,
            position: 0,
            created_at: 0,
            started_at: None,
            finished_at: None,
            expected_hash: None,
            checksum_algo: None,
            checksum_status: None,
            checksum_retries: 0,
            media: None,
        }
    }

    #[test]
    fn derives_package_status() {
        use DownloadStatus as D;
        let p = package(Stage::Download);
        assert_eq!(p.status(&[file(D::Downloading), file(D::Queued)]), PackageStatus::Downloading);
        assert_eq!(p.status(&[file(D::Failed), file(D::Completed)]), PackageStatus::Failed);
        assert_eq!(p.status(&[file(D::Failed), file(D::Queued)]), PackageStatus::Queued);
        assert_eq!(p.status(&[file(D::Paused), file(D::Completed)]), PackageStatus::Paused);
        assert_eq!(package(Stage::Remote).status(&[]), PackageStatus::Debrid);
        assert_eq!(package(Stage::Extract).status(&[]), PackageStatus::Extracting);
        assert_eq!(package(Stage::Verify).status(&[file(D::Completed)]), PackageStatus::Verifying);
    }
}
