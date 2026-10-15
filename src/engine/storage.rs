//! Destination folders: categories, automatic category by extension, package sub-folders, and
//! file cleanup.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use anyhow::Result;

use super::{Engine, naming};
use crate::{
    db::{self, packages::PackageUpdate},
    models::{CategoryRow, DownloadRow, DownloadStatus, PackageRow},
};

/// Absolute folder of a category: relative paths live under the download root.
pub fn category_path(root: &Path, category: &CategoryRow) -> PathBuf {
    let path = Path::new(category.path.trim());
    if path.as_os_str().is_empty() {
        root.to_path_buf()
    } else if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

/// Picks the category whose extensions match the most file names, falling back to the default
/// category (or the first one).
pub fn pick_category<'a>(categories: &'a [CategoryRow], file_names: &[&str]) -> Option<&'a CategoryRow> {
    let mut scores: HashMap<i64, usize> = HashMap::new();
    for name in file_names {
        let Some(ext) = naming::extension(name) else { continue };
        for category in categories {
            if category.extension_list().iter().any(|e| e.eq_ignore_ascii_case(&ext)) {
                *scores.entry(category.id).or_default() += 1;
            }
        }
    }
    let best = categories
        .iter()
        .filter(|c| scores.contains_key(&c.id))
        .max_by(|a, b| scores[&a.id].cmp(&scores[&b.id]).then(b.position.cmp(&a.position)));
    best.or_else(|| categories.iter().find(|c| c.is_default)).or_else(|| categories.first())
}

impl Engine {
    /// The package folder, resolved and stored the first time a file of the package starts.
    pub async fn package_dir(&self, package_id: i64) -> Result<PathBuf> {
        let _guard = self.dir_lock.lock().await;
        let package = db::packages::get(&self.db, package_id)
            .await?
            .ok_or_else(|| anyhow::anyhow!("package {package_id} not found"))?;
        if let Some(dir) = &package.dest_dir {
            return Ok(PathBuf::from(dir));
        }

        let categories = db::categories::list(&self.db).await?;
        let category = match package.category_id.and_then(|id| categories.iter().find(|c| c.id == id)) {
            Some(category) => Some(category),
            None => {
                let files = db::downloads::list_by_package(&self.db, package_id).await?;
                let names: Vec<&str> = files.iter().filter_map(|f| f.filename.as_deref()).collect();
                pick_category(&categories, &names)
            }
        };
        let base = match category {
            Some(category) => category_path(&self.config.download_root, category),
            None => self.config.download_root.clone(),
        };
        let dir = if package.create_subfolder { base.join(naming::sanitize(&package.name)) } else { base };

        db::packages::update(
            &self.db,
            package_id,
            PackageUpdate {
                dest_dir: Some(Some(dir.to_string_lossy().into_owned())),
                category_id: Some(package.category_id.or(category.map(|c| c.id))),
                ..Default::default()
            },
        )
        .await?;
        Ok(dir)
    }
}

pub async fn remove_temp_file(download: &DownloadRow) {
    let Some(temp) = &download.temp_path else { return };
    let temp = Path::new(temp);
    if super::media::is_work_dir(temp) {
        // A media download keeps its partial streams in a private folder.
        if let Err(error) = tokio::fs::remove_dir_all(temp).await
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("cannot delete {}: {error}", temp.display());
        }
    } else {
        remove_file(temp).await;
    }
}

/// Deletes the files a package downloaded, then its folder if it was dedicated and is empty.
/// Skipped files are left alone: they existed before and are not ours.
pub async fn remove_package_files(package: &PackageRow, files: &[DownloadRow]) {
    let downloaded = files.iter().filter(|f| f.status == DownloadStatus::Completed);
    for path in downloaded.filter_map(|f| f.path.as_deref()) {
        remove_file(Path::new(path)).await;
    }
    if package.create_subfolder
        && let Some(dir) = &package.dest_dir
    {
        remove_empty_dirs(Path::new(dir)).await;
    }
}

async fn remove_file(path: &Path) {
    match tokio::fs::remove_file(path).await {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => tracing::warn!("cannot delete {}: {error}", path.display()),
    }
}

/// Removes `dir` and its sub-folders if they contain no files.
async fn remove_empty_dirs(dir: &Path) {
    let dir = dir.to_path_buf();
    let _ = tokio::task::spawn_blocking(move || {
        fn prune(dir: &Path) -> bool {
            let Ok(entries) = std::fs::read_dir(dir) else { return false };
            let mut empty = true;
            for entry in entries.flatten() {
                let path = entry.path();
                if !(path.is_dir() && prune(&path)) {
                    empty = false;
                }
            }
            empty && std::fs::remove_dir(dir).is_ok()
        }
        prune(&dir)
    })
    .await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn category(id: i64, extensions: &str, is_default: bool) -> CategoryRow {
        CategoryRow {
            id,
            name: format!("c{id}"),
            path: format!("c{id}"),
            extensions: extensions.into(),
            is_default,
            position: id,
        }
    }

    #[test]
    fn picks_category_by_majority_extension() {
        let categories = vec![category(1, "", true), category(2, "mkv,mp4", false), category(3, "mp3,flac", false)];
        let pick = |names: &[&str]| pick_category(&categories, names).map(|c| c.id);
        assert_eq!(pick(&["a.mkv", "b.MKV", "c.mp3"]), Some(2));
        assert_eq!(pick(&["a.flac"]), Some(3));
        assert_eq!(pick(&["a.part1.rar"]), Some(1));
        assert_eq!(pick(&[]), Some(1));
    }

    #[test]
    fn resolves_relative_and_absolute_paths() {
        let root = Path::new("/downloads");
        let mut c = category(1, "", true);
        assert_eq!(category_path(root, &c), PathBuf::from("/downloads/c1"));
        c.path = String::new();
        assert_eq!(category_path(root, &c), PathBuf::from("/downloads"));
        #[cfg(unix)]
        {
            c.path = "/media/movies".into();
            assert_eq!(category_path(root, &c), PathBuf::from("/media/movies"));
        }
    }
}
