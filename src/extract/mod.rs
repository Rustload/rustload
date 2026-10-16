//! Archive extraction.
//!
//! Archives are grouped into *sets* (all volumes of one multi-part archive) from the package's own
//! files — never by listing the folder, so volumes of another package are never touched. Each set
//! is extracted into a private temporary folder, sanitised (no symlinks, normal permissions), then
//! moved next to the archive. RAR/7z/split archives use the 7-Zip CLI (`7zz`, `7z` or `7za`): the
//! copy installed from the interface in `<config>/bin/` (Windows), else the one in `PATH`. Plain ZIP
//! files fall back to a built-in extractor when 7-Zip is not installed.

mod sevenzip;
mod volumes;
mod zip;

use std::{
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use tokio_util::sync::CancellationToken;

pub use volumes::{ArchiveKind, ArchiveSet, find_archives};

use crate::{engine::naming, privileges};

#[derive(Debug, thiserror::Error)]
pub enum ExtractError {
    #[error("wrong password")]
    WrongPassword,
    /// The file has an archive extension but is not an archive.
    #[error("not an archive")]
    NotArchive,
    #[error("missing volume: {0}")]
    MissingVolume(String),
    #[error("{0}")]
    Failed(String),
    #[error("7-Zip is required to extract {0} archives")]
    NoExtractor(&'static str),
    #[error("cancelled")]
    Cancelled,
    #[error("disk error: {0}")]
    Io(#[from] std::io::Error),
}

/// Result of a successful extraction.
pub struct Extracted {
    /// Top-level entries created in the destination folder.
    pub entries: Vec<PathBuf>,
    /// Password that worked, if the archive was encrypted.
    pub password: Option<String>,
}

/// Progress callback, `0.0..=1.0`.
pub type Progress = Arc<dyn Fn(f64) + Send + Sync>;

#[derive(Debug)]
pub struct Extractor {
    /// Where Rustload installs 7-Zip, searched before `PATH`.
    bin_dir: PathBuf,
    seven_zip: RwLock<Option<PathBuf>>,
}

impl Extractor {
    pub fn detect(bin_dir: PathBuf) -> Self {
        let seven_zip = sevenzip::find(&bin_dir);
        Self { bin_dir, seven_zip: RwLock::new(seven_zip) }
    }

    /// Looks for 7-Zip again, e.g. once it was installed.
    pub fn refresh(&self) {
        *self.seven_zip.write().expect("poisoned") = sevenzip::find(&self.bin_dir);
    }

    /// Name of the external tool, shown in the UI.
    pub fn tool_name(&self) -> Option<String> {
        self.tool_path().and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
    }

    /// Path of the external tool.
    pub fn tool_path(&self) -> Option<PathBuf> {
        self.seven_zip.read().expect("poisoned").clone()
    }

    /// Extracts `set` next to its first volume, trying `passwords` in order (no password first).
    pub async fn extract(
        &self,
        set: &ArchiveSet,
        passwords: &[String],
        progress: Progress,
        cancel: &CancellationToken,
    ) -> Result<Extracted, ExtractError> {
        if let Some(missing) = set.missing_volume() {
            return Err(ExtractError::MissingVolume(missing));
        }
        let dest = set.first.parent().map(Path::to_path_buf).unwrap_or_default();
        let staging =
            dest.join(format!(".rustload-extract-{}", crate::util::random_token().get(..12).unwrap_or("tmp")));

        let mut candidates: Vec<Option<&str>> = vec![None];
        candidates.extend(passwords.iter().map(|p| Some(p.as_str())));

        let seven_zip = self.tool_path();
        let mut result = Err(ExtractError::WrongPassword);
        for password in candidates {
            reset_dir(&staging).await?;
            result = match &seven_zip {
                Some(tool) => sevenzip::extract(tool, &set.first, &staging, password, progress.clone(), cancel).await,
                None if set.kind == ArchiveKind::Zip => {
                    zip::extract(&set.first, &staging, password, progress.clone()).await
                }
                None => Err(ExtractError::NoExtractor(set.kind.label())),
            }
            .map(|()| password.map(str::to_owned));
            if !matches!(result, Err(ExtractError::WrongPassword)) {
                break;
            }
        }

        let outcome = match result {
            Ok(password) => {
                let staging_clone = staging.clone();
                let dest_clone = dest.clone();
                let entries = tokio::task::spawn_blocking(move || publish(&staging_clone, &dest_clone))
                    .await
                    .map_err(|e| ExtractError::Failed(e.to_string()))??;
                Ok(Extracted { entries, password })
            }
            Err(error) => Err(error),
        };
        let _ = tokio::fs::remove_dir_all(&staging).await;
        outcome
    }
}

async fn reset_dir(dir: &Path) -> std::io::Result<()> {
    match tokio::fs::remove_dir_all(dir).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    tokio::fs::create_dir_all(dir).await
}

/// Sanitises the staging folder and moves its content into `dest`, renaming on conflicts.
fn publish(staging: &Path, dest: &Path) -> Result<Vec<PathBuf>, ExtractError> {
    sanitize_tree(staging)?;
    let mut entries = Vec::new();
    for entry in std::fs::read_dir(staging)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let target = naming::unique_path(dest, &name);
        std::fs::rename(entry.path(), &target)?;
        entries.push(target);
    }
    Ok(entries)
}

/// Removes symlinks (they could point outside the download folder) and gives every file the
/// permissions a freshly created one would have.
fn sanitize_tree(dir: &Path) -> Result<(), ExtractError> {
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            tracing::warn!("removed symlink from archive: {}", path.display());
            std::fs::remove_file(&path)?;
        } else if file_type.is_dir() {
            privileges::normalize_permissions(&path, true)?;
            sanitize_tree(&path)?;
        } else {
            privileges::normalize_permissions(&path, false)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use super::*;

    #[tokio::test]
    async fn extracts_zip_without_external_tool() {
        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("pack.zip");
        {
            let file = std::fs::File::create(&archive).unwrap();
            let mut writer = ::zip::ZipWriter::new(file);
            let options = ::zip::write::SimpleFileOptions::default();
            writer.start_file("folder/hello.txt", options).unwrap();
            writer.write_all(b"hello").unwrap();
            writer.finish().unwrap();
        }
        // Something with the same name already exists: the extracted folder gets renamed.
        std::fs::create_dir(dir.path().join("folder")).unwrap();

        let sets = find_archives(std::slice::from_ref(&archive));
        assert_eq!(sets.len(), 1);
        let extractor = Extractor::with_tool(None);
        let result = extractor.extract(&sets[0], &[], Arc::new(|_| {}), &CancellationToken::new()).await.unwrap();
        assert_eq!(result.entries, vec![dir.path().join("folder (1)")]);
        assert_eq!(std::fs::read_to_string(dir.path().join("folder (1)/hello.txt")).unwrap(), "hello");
        assert_staging_removed(dir.path());
    }

    /// Runs only when a 7-Zip binary is installed.
    #[tokio::test]
    async fn extracts_split_encrypted_7z_with_7zip() {
        let Some(tool) = sevenzip::find(&std::env::temp_dir()) else { return };
        let dir = tempfile::tempdir().unwrap();
        let source = dir.path().join("source");
        std::fs::create_dir(&source).unwrap();
        let payload: Vec<u8> = (0..3_000_000u32).map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8).collect();
        std::fs::write(source.join("movie.mkv"), &payload).unwrap();
        let status = std::process::Command::new(&tool)
            .args(["a", "-t7z", "-mx=0", "-v1m", "-psecret", "-mhe=on"])
            .arg(dir.path().join("pack.7z"))
            .arg(source.join("movie.mkv"))
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success());
        std::fs::remove_dir_all(&source).unwrap();

        let volumes: Vec<PathBuf> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().path()).collect();
        let sets = find_archives(&volumes);
        assert_eq!(sets.len(), 1);
        assert!(sets[0].volumes.len() >= 3, "expected several volumes");

        let extractor = Extractor::with_tool(Some(tool));
        let result = extractor
            .extract(&sets[0], &["wrong".into(), "secret".into()], Arc::new(|_| {}), &CancellationToken::new())
            .await
            .unwrap();
        assert_eq!(result.password.as_deref(), Some("secret"));
        assert_eq!(std::fs::read(dir.path().join("movie.mkv")).unwrap(), payload);
        assert_staging_removed(dir.path());
    }

    impl Extractor {
        fn with_tool(seven_zip: Option<PathBuf>) -> Self {
            Self { bin_dir: PathBuf::new(), seven_zip: RwLock::new(seven_zip) }
        }
    }

    fn assert_staging_removed(dir: &Path) {
        let leftovers: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| e.file_name().to_string_lossy().starts_with(".rustload"))
            .collect();
        assert!(leftovers.is_empty());
    }
}
