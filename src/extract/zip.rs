//! Built-in ZIP extraction, used when 7-Zip is not installed.

use std::{
    fs::File,
    io::{self, BufReader},
    path::{Path, PathBuf},
};

use zip::{ZipArchive, result::ZipError};

use super::{ExtractError, Progress};

pub async fn extract(
    archive: &Path,
    dest: &Path,
    password: Option<&str>,
    progress: Progress,
) -> Result<(), ExtractError> {
    let (archive, dest, password) = (archive.to_path_buf(), dest.to_path_buf(), password.map(str::to_owned));
    tokio::task::spawn_blocking(move || extract_blocking(&archive, &dest, password.as_deref(), &progress))
        .await
        .map_err(|e| ExtractError::Failed(e.to_string()))?
}

fn extract_blocking(
    archive: &Path,
    dest: &Path,
    password: Option<&str>,
    progress: &Progress,
) -> Result<(), ExtractError> {
    let mut zip = ZipArchive::new(BufReader::new(File::open(archive)?)).map_err(zip_error)?;
    let total = zip.len().max(1);
    for index in 0..zip.len() {
        let mut entry = match password {
            Some(password) => zip.by_index_decrypt(index, password.as_bytes()),
            None => zip.by_index(index),
        }
        .map_err(zip_error)?;
        // `enclosed_name` rejects absolute paths and `..` traversal.
        let Some(relative) = entry.enclosed_name() else {
            tracing::warn!("skipped unsafe path in archive: {}", String::from_utf8_lossy(entry.name_raw()));
            continue;
        };
        let target: PathBuf = dest.join(relative);
        if entry.is_dir() {
            std::fs::create_dir_all(&target)?;
        } else if entry.is_symlink() {
            tracing::warn!("skipped symlink in archive: {}", String::from_utf8_lossy(entry.name_raw()));
        } else {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            let mut out = File::create(&target)?;
            io::copy(&mut entry, &mut out).map_err(|e| match e.kind() {
                io::ErrorKind::InvalidData | io::ErrorKind::InvalidInput if password.is_some() => {
                    ExtractError::WrongPassword
                }
                _ => ExtractError::Io(e),
            })?;
        }
        progress((index + 1) as f64 / total as f64);
    }
    Ok(())
}

fn zip_error(error: ZipError) -> ExtractError {
    match error {
        ZipError::InvalidPassword => ExtractError::WrongPassword,
        ZipError::UnsupportedArchive(message) if message == ZipError::PASSWORD_REQUIRED => ExtractError::WrongPassword,
        ZipError::Io(error) => ExtractError::Io(error),
        ZipError::InvalidArchive(_) => ExtractError::NotArchive,
        other => ExtractError::Failed(other.to_string()),
    }
}
