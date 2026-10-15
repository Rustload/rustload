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

pub async fn run(engine: Arc<Engine>) {
    let mut seen: HashMap<PathBuf, (u64, SystemTime)> = HashMap::new();
    loop {
        tokio::select! {
            _ = engine.shutdown_token().cancelled() => return,
            _ = tokio::time::sleep(SCAN_INTERVAL) => {}
        }
        let settings = engine.settings.get();
        if !settings.watch.enabled {
            seen.clear();
            continue;
        }
        let dir = resolve_dir(&engine.config.config_dir, &settings.watch.dir);
        let options = AddOptions { category_id: settings.watch.category_id, ..Default::default() };
        for path in ready_files(&dir, &mut seen).await {
            import(&engine, &dir, &path, &options).await;
        }
    }
}

fn resolve_dir(config_dir: &Path, dir: &str) -> PathBuf {
    let path = Path::new(dir);
    if path.is_absolute() { path.to_path_buf() } else { config_dir.join(path) }
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
