//! Destination folders: categories, automatic category by rule and extension, package sub-folders,
//! and file cleanup.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use regex::{Regex, RegexBuilder};

use super::{Engine, naming, task::TaskError};
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
        // On Windows `/series` is not absolute (no drive) but `join` would still put it at the root
        // of the drive: like the interface, keep it below the download root.
        root.join(path.to_string_lossy().trim_start_matches(['/', '\\']))
    }
}

/// The folder of a category outside the download root (an absolute path: a Docker volume, a network
/// share) when it does not exist. Rustload creates the folders below the download root, never this
/// one: writing to an unmounted volume would fill the container or the local disk instead.
pub async fn missing_mount(root: &Path, category: &CategoryRow) -> Option<PathBuf> {
    if !Path::new(category.path.trim()).is_absolute() {
        return None;
    }
    let dir = category_path(root, category);
    match tokio::fs::metadata(&dir).await {
        Ok(metadata) if metadata.is_dir() => None,
        _ => Some(dir),
    }
}

/// Compiled size limit of a rule: rules are short expressions, this only stops pathological ones.
const RULE_SIZE_LIMIT: usize = 1 << 20;

/// Compiles the rule of a category: a case-insensitive regular expression found anywhere in the
/// text (`jeu\.fr`, `S\d{2}E\d{2}`).
pub fn rule(pattern: &str) -> Result<Regex, regex::Error> {
    RegexBuilder::new(pattern).case_insensitive(true).size_limit(RULE_SIZE_LIMIT).build()
}

/// Picks the category of a package in automatic mode, in this order:
/// 1. the first category, in list order, whose rule matches one of `texts` (package name, links)
///    or `file_names`;
/// 2. the category whose extensions match the most file names;
/// 3. the default category (or the first one).
pub fn pick_category<'a>(
    categories: &'a [CategoryRow],
    texts: &[&str],
    file_names: &[&str],
) -> Option<&'a CategoryRow> {
    let ruled = categories.iter().find(|category| {
        let pattern = category.pattern.trim();
        !pattern.is_empty()
            && rule(pattern).is_ok_and(|rule| texts.iter().chain(file_names).any(|text| rule.is_match(text)))
    });
    if ruled.is_some() {
        return ruled;
    }

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
    /// `starting` is that file (download id and name), whose name may not be stored yet.
    pub async fn package_dir(&self, package_id: i64, starting: Option<(i64, &str)>) -> Result<PathBuf, TaskError> {
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
                // Files not checked yet have no name stored: the starting one knows its own, the
                // others have the name in their link.
                let file_names: Vec<String> = files
                    .iter()
                    .filter_map(|f| match starting {
                        Some((id, name)) if id == f.id => Some(name.to_owned()),
                        _ => f.filename.clone().or_else(|| naming::from_url(&f.url)),
                    })
                    .collect();
                let file_names: Vec<&str> = file_names.iter().map(String::as_str).collect();
                let texts: Vec<&str> = [package.name.as_str()]
                    .into_iter()
                    .chain(package.source.as_deref())
                    .chain(files.iter().map(|f| f.url.as_str()))
                    .collect();
                pick_category(&categories, &texts, &file_names)
            }
        };
        if let Some(category) = category
            && let Some(dir) = missing_mount(&self.config.download_root, category).await
        {
            // Not stored: the next attempt resolves the folder again, once it is mounted.
            let fix = if self.config.desktop {
                "create it or connect the drive it is on"
            } else {
                "mount it in the container or create it"
            };
            return Err(TaskError::Fatal(format!(
                "the folder {} of the category {} does not exist: {fix}, then retry",
                dir.display(),
                category.name
            )));
        }
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
            pattern: String::new(),
        }
    }

    #[test]
    fn picks_category_by_majority_extension() {
        let categories = vec![category(1, "", true), category(2, "mkv,mp4", false), category(3, "mp3,flac", false)];
        let pick = |names: &[&str]| pick_category(&categories, &[], names).map(|c| c.id);
        assert_eq!(pick(&["a.mkv", "b.MKV", "c.mp3"]), Some(2));
        assert_eq!(pick(&["a.flac"]), Some(3));
        assert_eq!(pick(&["a.part1.rar"]), Some(1));
        assert_eq!(pick(&[]), Some(1));
    }

    #[test]
    fn rules_come_before_extensions_in_list_order() {
        let mut games = category(2, "msi", false);
        games.pattern = r"jeu\.fr".into();
        let mut shows = category(3, "", false);
        shows.pattern = r"S\d{2}E\d{2}".into();
        let mut broken = category(4, "", false);
        broken.pattern = "(".into();
        let categories = vec![category(1, "mkv", true), games, shows, broken];
        let pick = |texts: &[&str], names: &[&str]| pick_category(&categories, texts, names).map(|c| c.id);

        // The link matches the rule of Games, whatever the files are.
        assert_eq!(pick(&["https://www.JEU.fr/dl/123"], &["game.part1.rar"]), Some(2));
        // File names are matched too, case-insensitively, and win over the extensions.
        assert_eq!(pick(&["https://host.example/x"], &["Show.s01e02.mkv"]), Some(3));
        // Both rules match: the first in the list wins.
        assert_eq!(pick(&["jeu.fr Show S01E02"], &[]), Some(2));
        // No rule matches: extensions, then the default. An invalid rule never matches.
        assert_eq!(pick(&["https://host.example/x"], &["setup.msi"]), Some(2));
        assert_eq!(pick(&["("], &["a.zip"]), Some(1));
    }

    #[test]
    fn validates_rules() {
        assert!(rule(r"jeu\.fr").unwrap().is_match("https://JEU.FR/a"));
        assert!(rule("(").is_err());
        assert!(rule(r"\w{1000}{1000}").is_err());
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

    /// Only a drive or a network share is absolute on Windows: `/series` stays below the root,
    /// where the interface shows it, instead of `C:\series`.
    #[cfg(windows)]
    #[test]
    fn keeps_rooted_paths_below_the_download_root_on_windows() {
        let root = Path::new(r"C:\Users\me\Downloads\Rustload");
        let mut c = category(1, "", true);
        c.path = "/series".into();
        assert_eq!(category_path(root, &c), root.join("series"));
        c.path = r"\films\new".into();
        assert_eq!(category_path(root, &c), root.join(r"films\new"));
        c.path = r"D:\Films".into();
        assert_eq!(category_path(root, &c), PathBuf::from(r"D:\Films"));
        c.path = r"\\nas\media".into();
        assert_eq!(category_path(root, &c), PathBuf::from(r"\\nas\media"));
    }
}
