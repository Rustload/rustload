//! Watch folder ("hot folder"): `.torrent`, `.magnet`, `.txt` and `.links` files dropped in the
//! folder are imported, then moved to `.imported/` or `.failed/`.
//!
//! Text files: one link per line, `;` starts a comment and `[Name]` starts a
//! new package.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, SystemTime},
};

use crate::{
    config::Config,
    engine::{
        Engine,
        intake::{self, AddOptions},
        naming,
    },
    events::{Notice, NoticeLevel},
};

const SCAN_INTERVAL: Duration = Duration::from_secs(5);
/// A file must not change for this long before it is imported (still being written otherwise).
const SETTLE_TIME: Duration = Duration::from_secs(3);

/// The watch folder of the Docker image, the default setting.
const DOCKER_DIR: &str = "/watch";

pub async fn run(engine: Arc<Engine>) {
    let mut seen: HashMap<PathBuf, (u64, SystemTime)> = HashMap::new();
    // The desktop app creates its folder: the last one it tried.
    let mut prepared: Option<PathBuf> = None;
    loop {
        tokio::select! {
            _ = engine.shutdown_token().cancelled() => return,
            _ = tokio::time::sleep(SCAN_INTERVAL) => {}
        }
        let settings = engine.settings.get();
        if !settings.watch.enabled {
            seen.clear();
            prepared = None;
            continue;
        }
        let dir = resolve_dir(&engine.config, &settings.watch.dir);
        if engine.config.desktop && prepared.as_ref() != Some(&dir) {
            prepare(&engine, &dir).await;
            prepared = Some(dir.clone());
        }
        let options = AddOptions { category_id: settings.watch.category_id, ..Default::default() };
        for path in ready_files(&dir, &mut seen).await {
            import(&engine, &dir, &path, &options).await;
        }
    }
}

/// A server reads relative folders from its config folder. The desktop app has no Docker volume:
/// the default and relative folders live in its download root (`Watch` by default).
fn resolve_dir(config: &Config, dir: &str) -> PathBuf {
    let dir = dir.trim();
    let path = Path::new(dir);
    if config.desktop {
        if dir.is_empty() || dir == DOCKER_DIR {
            return config.download_root.join("Watch");
        }
        if !path.is_absolute() {
            return config.download_root.join(dir.trim_start_matches(['/', '\\']));
        }
    }
    if path.is_absolute() { path.to_path_buf() } else { config.config_dir.join(path) }
}

/// Creates the desktop app's watch folder, or says why it cannot.
async fn prepare(engine: &Engine, dir: &Path) {
    if let Err(error) = tokio::fs::create_dir_all(dir).await {
        let message = format!("cannot create the watch folder {}: {error}", dir.display());
        tracing::warn!("{message}");
        engine.events.notice(
            Notice::new(NoticeLevel::Warning, "watch_folder_unavailable", message)
                .param("dir", dir.display().to_string())
                .param("error", error.to_string()),
        );
    }
}

/// Files that did not change since the previous scan and are old enough.
async fn ready_files(dir: &Path, seen: &mut HashMap<PathBuf, (u64, SystemTime)>) -> Vec<PathBuf> {
    let Ok(mut entries) = tokio::fs::read_dir(dir).await else { return Vec::new() };
    let mut ready = Vec::new();
    let mut current = HashMap::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let path = entry.path();
        let hidden = entry.file_name().to_string_lossy().starts_with('.');
        let Ok(meta) = entry.metadata().await else { continue };
        if hidden || !meta.is_file() || !is_supported(&path) {
            continue;
        }
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let fingerprint = (meta.len(), modified);
        let settled = modified.elapsed().is_ok_and(|age| age >= SETTLE_TIME);
        if settled && seen.get(&path) == Some(&fingerprint) {
            ready.push(path.clone());
        }
        current.insert(path, fingerprint);
    }
    *seen = current;
    ready
}

fn is_supported(path: &Path) -> bool {
    matches!(
        path.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).as_deref(),
        Some("torrent" | "magnet" | "txt" | "links")
    )
}

async fn import(engine: &Engine, dir: &Path, path: &Path, options: &AddOptions) {
    let file_name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let is_torrent = path.extension().is_some_and(|e| e.eq_ignore_ascii_case("torrent"));
    let result = match tokio::fs::read(path).await {
        Ok(data) if is_torrent => intake::add_torrent(engine, &file_name, data, options).await.map(|id| vec![id]),
        Ok(data) => add_sections(engine, &String::from_utf8_lossy(&data), options).await,
        Err(error) => Err(intake::IntakeError::Internal(error.into())),
    };

    let (subdir, outcome) = match &result {
        Ok(ids) => {
            tracing::info!("watch folder: imported {file_name} ({} package(s))", ids.len());
            engine.events.notice(
                Notice::new(NoticeLevel::Info, "watch_imported", format!("Imported {file_name}"))
                    .param("file", &file_name)
                    .param("count", ids.len().to_string()),
            );
            (".imported", "imported")
        }
        Err(error) => {
            tracing::warn!("watch folder: cannot import {file_name}: {error}");
            (".failed", "failed")
        }
    };
    let target_dir = dir.join(subdir);
    let moved = async {
        tokio::fs::create_dir_all(&target_dir).await?;
        tokio::fs::rename(path, naming::unique_path(&target_dir, &file_name)).await
    };
    if let Err(error) = moved.await {
        tracing::warn!("watch folder: cannot move {outcome} file {file_name}: {error}");
        // Never import the same file twice.
        let _ = tokio::fs::remove_file(path).await;
    }
}

async fn add_sections(engine: &Engine, text: &str, options: &AddOptions) -> Result<Vec<i64>, intake::IntakeError> {
    let mut ids = Vec::new();
    for (name, links) in parse_sections(text) {
        let options = AddOptions { name: name.or_else(|| options.name.clone()), ..options.clone() };
        ids.extend(intake::add_text(engine, &links, &options).await?);
    }
    if ids.is_empty() { Err(intake::IntakeError::NoLinks) } else { Ok(ids) }
}

/// Splits a link list into `[Name]` sections. Links before the first header have no name.
pub fn parse_sections(text: &str) -> Vec<(Option<String>, String)> {
    let mut sections: Vec<(Option<String>, String)> = vec![(None, String::new())];
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push((Some(name.trim().to_owned()), String::new()));
        } else if let Some((_, links)) = sections.last_mut() {
            links.push_str(line);
            links.push('\n');
        }
    }
    sections.into_iter().filter(|(_, links)| !links.is_empty()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_the_desktop_apps_folder_in_its_download_root() {
        let dir = tempfile::tempdir().unwrap();
        let mut config = Config::for_tests(dir.path());
        config.desktop = true;
        let root = config.download_root.clone();
        assert_eq!(resolve_dir(&config, DOCKER_DIR), root.join("Watch"));
        assert_eq!(resolve_dir(&config, " "), root.join("Watch"));
        assert_eq!(resolve_dir(&config, "drop"), root.join("drop"));
        let elsewhere = dir.path().join("elsewhere");
        assert_eq!(resolve_dir(&config, elsewhere.to_str().unwrap()), elsewhere);

        config.desktop = false;
        assert_eq!(resolve_dir(&config, "drop"), config.config_dir.join("drop"));
        #[cfg(unix)]
        assert_eq!(resolve_dir(&config, DOCKER_DIR), PathBuf::from(DOCKER_DIR));
    }

    #[test]
    fn parses_text_sections() {
        let text = "https://a.com/1\n; comment\n[Movie]\nhttps://b.com/2\n\n[Empty]\n[Show]\nmagnet:?xt=urn:btih:x\n";
        let sections = parse_sections(text);
        assert_eq!(
            sections,
            vec![
                (None, "https://a.com/1\n".into()),
                (Some("Movie".into()), "https://b.com/2\n".into()),
                (Some("Show".into()), "magnet:?xt=urn:btih:x\n".into()),
            ]
        );
    }
}
