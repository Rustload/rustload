//! Checksum verification stage. Once every file of a package is downloaded, the files are checked
//! against the checksum files of the package (`.sfv`, `.md5`, `.sha1`, `.sha256`, `.sha512`) and
//! the digests published by providers, before extraction. A corrupted file is deleted and
//! downloaded again once; if it is still corrupted, it fails. A file that cannot be read fails too:
//! nothing passes unchecked.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::{Arc, atomic::Ordering},
    time::Duration,
};

use anyhow::Result;

use super::{
    Engine,
    lifecycle::{self, Settled},
    live::Extraction,
};
use crate::{
    checksum::{self, FileHash, HashAlgo},
    db::{self, downloads::DownloadUpdate},
    events::{Notice, NoticeLevel},
    integrations::notify::Message,
    models::{DownloadRow, DownloadStatus, Stage},
    settings::NotificationEvent,
};

/// Values of `downloads.checksum_status`.
pub const OK: &str = "ok";
pub const MISMATCH: &str = "mismatch";
/// Fresh downloads of a corrupted file before it fails.
const MAX_RETRIES: i64 = 1;
/// Bigger files are not checksum files, whatever their extension.
const MAX_CHECKSUM_FILE: u64 = 4 * 1024 * 1024;
/// Pause before verifying a package again after an unexpected error (database...).
const RETRY_DELAY: Duration = Duration::from_secs(60);

/// Whether a package with these files may have something to verify. Cheap (no file access), so it
/// can be asked every time a file settles.
pub fn has_work(files: &[DownloadRow]) -> bool {
    completed(files).any(|(file, path)| {
        (file.expected_hash.is_some() && file.checksum_status.as_deref() != Some(OK)) || is_checksum_file(&path)
    })
}

fn completed(files: &[DownloadRow]) -> impl Iterator<Item = (&DownloadRow, PathBuf)> {
    files.iter().filter_map(|f| Some((f, f.completed_path()?)))
}

fn is_checksum_file(path: &Path) -> bool {
    path.file_name().and_then(|name| HashAlgo::of_checksum_file(&name.to_string_lossy())).is_some()
}

impl Engine {
    /// One verification at a time: hashing is disk-bound and parallel runs only thrash.
    pub(super) async fn spawn_verifications(self: &Arc<Self>) -> Result<()> {
        if !self.live.verifying_ids().is_empty() {
            return Ok(());
        }
        if let Some(package) = db::packages::list_by_stage(&self.db, Stage::Verify).await?.into_iter().next() {
            let verification = self.live.start_verification(package.id, &self.shutdown);
            self.tasks.spawn(run(self.clone(), package.id, verification));
        }
        Ok(())
    }
}

async fn run(engine: Arc<Engine>, package_id: i64, verification: Extraction) {
    let result = async {
        match verify_package(&engine, package_id, &verification).await? {
            Some(outcome) => conclude(&engine, package_id, outcome).await,
            // Package deleted, or shutdown: the stage is kept and verification restarts on next boot.
            None => Ok(()),
        }
    };
    if let Err(error) = result.await {
        // Not a verdict on the files: the package stays in the verify stage and is verified again
        // later (holding the slot meanwhile, so that a persistent error does not loop).
        tracing::error!(
            "cannot verify the checksums of package {package_id}: {error:#} (retrying in {} s)",
            RETRY_DELAY.as_secs()
        );
        tokio::select! {
            _ = verification.cancel.cancelled() => {}
            _ = tokio::time::sleep(RETRY_DELAY) => {}
        }
    }
    // Released only now: the scheduler must not see the package still in the verify stage without
    // a running verification, or it would start a second one.
    engine.live.finish_verification(package_id);
    engine.events.queue_changed();
    engine.wake();
}

#[derive(Debug, Default)]
struct Outcome {
    /// Corrupted or unreadable files, queued again or failed.
    rejected: usize,
}

/// A file to verify and the digest it must have.
struct Check {
    id: i64,
    name: String,
    path: PathBuf,
    hash: FileHash,
    retries: i64,
}

/// Verifies the files of a package; `None` when cancelled.
async fn verify_package(engine: &Engine, package_id: i64, verification: &Extraction) -> Result<Option<Outcome>> {
    let Some(package) = db::packages::get(&engine.db, package_id).await? else { return Ok(None) };
    let mut outcome = Outcome::default();
    if !engine.settings.get().postprocess.verify_checksums {
        return Ok(Some(outcome));
    }
    let files = db::downloads::list_by_package(&engine.db, package_id).await?;
    let checks = plan(&files).await;
    let mut sizes = Vec::with_capacity(checks.len());
    for check in &checks {
        sizes.push(tokio::fs::metadata(&check.path).await.map(|m| m.len()).unwrap_or(0));
    }
    let total = sizes.iter().sum::<u64>().max(1);
    let mut done = 0;

    for (check, size) in checks.into_iter().zip(sizes) {
        let (cancel, progress, path) = (verification.cancel.clone(), verification.progress.clone(), check.path.clone());
        let algo = check.hash.algo;
        let hashed = tokio::task::spawn_blocking(move || {
            let mut read = 0;
            checksum::hash_file(&path, algo, &cancel, &mut |bytes| {
                read += bytes;
                progress.store(((done + read) * 1000 / total).min(1000) as u32, Ordering::Relaxed);
            })
        })
        .await?;
        done += size;
        match hashed {
            Ok(Some(digest)) if digest == check.hash.hex => {
                tracing::info!("{}: {algo} checksum ok", check.name);
                record(engine, check.id, algo, OK).await?;
            }
            Ok(Some(_)) => {
                outcome.rejected += 1;
                reject(engine, &package.name, &check).await?;
            }
            Ok(None) => return Ok(None),
            Err(error) => {
                outcome.rejected += 1;
                unreadable(engine, &package.name, &check, &error).await?;
            }
        }
    }
    Ok(Some(outcome))
}

/// Moves the package on: to extraction when every file is downloaded (and verified), else back to
/// downloading, for the files rejected now or by an earlier run that was interrupted before it
/// could move the package.
async fn conclude(engine: &Engine, package_id: i64, outcome: Outcome) -> Result<()> {
    let Some(package) = db::packages::get(&engine.db, package_id).await? else { return Ok(()) };
    let files = db::downloads::list_by_package(&engine.db, package_id).await?;
    if outcome.rejected == 0 && files.iter().all(|f| f.status.is_done()) {
        return lifecycle::after_download(engine, &package, &files, Stage::Verify).await;
    }
    if !db::packages::advance(&engine.db, package_id, Stage::Verify, Stage::Download).await? {
        return Ok(());
    }
    if outcome.rejected == 0 {
        tracing::info!("{}: files rejected by an interrupted verification are downloaded again", package.name);
    }
    if files.iter().any(|f| f.status == DownloadStatus::Failed) {
        // Reports the package as failed unless other files are still to download.
        lifecycle::on_file_settled(engine, package_id, Settled::Rejected).await?;
    }
    Ok(())
}

async fn record(engine: &Engine, id: i64, algo: HashAlgo, status: &str) -> Result<()> {
    let update = DownloadUpdate {
        checksum_algo: Some(Some(algo.name().to_owned())),
        checksum_status: Some(Some(status.to_owned())),
        ..Default::default()
    };
    db::downloads::update(&engine.db, id, update).await
}

/// Deletes a corrupted file and queues it again, or fails it after [`MAX_RETRIES`] fresh
/// downloads.
async fn reject(engine: &Engine, package_name: &str, check: &Check) -> Result<()> {
    let again = check.retries < MAX_RETRIES;
    let algo = check.hash.algo;
    if let Err(error) = tokio::fs::remove_file(&check.path).await
        && error.kind() != std::io::ErrorKind::NotFound
    {
        tracing::warn!("cannot delete {}: {error}", check.path.display());
    }
    let (status, error) = if again {
        (DownloadStatus::Queued, format!("checksum mismatch ({algo}), downloading again"))
    } else {
        (DownloadStatus::Failed, format!("checksum mismatch ({algo})"))
    };
    let update = DownloadUpdate {
        status: Some(status),
        error: Some(Some(error.clone())),
        path: Some(None),
        downloaded: Some(0),
        temp_path: Some(None),
        resume: Some(None),
        attempts: Some(0),
        retry_at: Some(None),
        finished_at: Some(None),
        checksum_algo: Some(Some(algo.name().to_owned())),
        checksum_status: Some(Some(MISMATCH.to_owned())),
        checksum_retries: Some(check.retries + i64::from(again)),
        ..Default::default()
    };
    db::downloads::update(&engine.db, check.id, update).await?;
    if !again {
        engine.stats.file_failed().await;
    }

    let detail = format!("{}: {error}", check.name);
    tracing::warn!("{package_name}: {detail}");
    let level = if again { NoticeLevel::Warning } else { NoticeLevel::Error };
    engine.events.notice(
        Notice::new(level, "checksum_failed", format!("{package_name}: {detail}"))
            .param("name", package_name)
            .param("file", &check.name)
            .param("algo", algo.name()),
    );
    let settings = engine.settings.get();
    let event = NotificationEvent::ChecksumFailed;
    engine.notifier.send(
        &settings.notifications.targets,
        event,
        Message::for_package(event, package_name, 0, Some(&detail)),
    );
    Ok(())
}

/// Fails a file that cannot be read to verify it (deleted or moved meanwhile, permissions...),
/// leaving it as it is: retrying it downloads it again.
async fn unreadable(engine: &Engine, package_name: &str, check: &Check, error: &std::io::Error) -> Result<()> {
    let message = format!("cannot read the file to verify its checksum: {error}");
    let update = DownloadUpdate {
        status: Some(DownloadStatus::Failed),
        error: Some(Some(message.clone())),
        retry_at: Some(None),
        checksum_algo: Some(Some(check.hash.algo.name().to_owned())),
        checksum_status: Some(None),
        ..Default::default()
    };
    db::downloads::update(&engine.db, check.id, update).await?;
    engine.stats.file_failed().await;
    tracing::warn!("{package_name}: {}: {message} ({})", check.name, check.path.display());
    engine.events.notice(
        Notice::new(NoticeLevel::Error, "download_failed", format!("{} failed: {message}", check.name))
            .param("name", &check.name)
            .param("error", &message),
    );
    Ok(())
}

/// Expected digests of the completed files not verified yet. Several sources may describe a file
/// (checksum files, provider digest): the strongest algorithm wins.
async fn plan(files: &[DownloadRow]) -> Vec<Check> {
    let completed: Vec<(&DownloadRow, PathBuf)> = completed(files).collect();
    let mut expected: HashMap<i64, FileHash> = HashMap::new();
    let mut offer = |id: i64, hash: FileHash| match expected.get(&id) {
        Some(current) if current.algo >= hash.algo => {}
        _ => {
            expected.insert(id, hash);
        }
    };

    for (file, _) in &completed {
        if let Some(hash) = file.expected_hash.as_deref().and_then(|h| h.parse().ok()) {
            offer(file.id, hash);
        }
    }
    let targets: Vec<(i64, &Path)> = completed.iter().map(|(file, path)| (file.id, path.as_path())).collect();
    for (_, path) in completed.iter().filter(|(_, path)| is_checksum_file(path)) {
        let (Some(name), Some(dir)) = (path.file_name(), path.parent()) else { continue };
        let Some(content) = read_checksum_file(path).await else { continue };
        for entry in checksum::parse_checksum_file(&name.to_string_lossy(), &content) {
            if let Some(id) = find_target(&targets, dir, &entry.path) {
                offer(id, entry.hash);
            }
        }
    }

    completed
        .into_iter()
        .filter(|(file, _)| file.checksum_status.as_deref() != Some(OK))
        .filter_map(|(file, path)| {
            let hash = expected.remove(&file.id)?;
            let name = file
                .filename
                .clone()
                .or_else(|| path.file_name().map(|n| n.to_string_lossy().into_owned()))
                .unwrap_or_else(|| path.display().to_string());
            Some(Check { id: file.id, name, path, hash, retries: file.checksum_retries })
        })
        .collect()
}

async fn read_checksum_file(path: &Path) -> Option<String> {
    let size = tokio::fs::metadata(path).await.ok()?.len();
    if size > MAX_CHECKSUM_FILE {
        return None;
    }
    match tokio::fs::read(path).await {
        Ok(bytes) => Some(String::from_utf8_lossy(&bytes).into_owned()),
        Err(error) => {
            tracing::warn!("cannot read {}: {error}", path.display());
            None
        }
    }
}

/// The file an entry of a checksum file in `dir` refers to: same relative path (ignoring case,
/// names often differ in case between Windows and the hoster), or else the only file with that
/// name when the entry has no folder.
fn find_target(files: &[(i64, &Path)], dir: &Path, entry: &str) -> Option<i64> {
    let wanted = entry.to_lowercase();
    let relative = |path: &Path| {
        let rest = path.strip_prefix(dir).ok()?;
        let parts: Vec<String> = rest.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        Some(parts.join("/").to_lowercase())
    };
    if let Some((id, _)) = files.iter().find(|(_, path)| relative(path).as_deref() == Some(wanted.as_str())) {
        return Some(*id);
    }
    if wanted.contains('/') {
        return None;
    }
    let mut same_name =
        files.iter().filter(|(_, path)| path.file_name().is_some_and(|n| n.to_string_lossy().to_lowercase() == wanted));
    match (same_name.next(), same_name.next()) {
        (Some((id, _)), None) => Some(*id),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(id: i64, path: &Path, expected_hash: Option<&str>) -> DownloadRow {
        DownloadRow {
            id,
            package_id: 1,
            url: format!("https://example.com/{id}"),
            rel_dir: String::new(),
            filename: path.file_name().map(|n| n.to_string_lossy().into_owned()),
            size: None,
            downloaded: 0,
            status: DownloadStatus::Completed,
            error: None,
            via: None,
            host: None,
            account_id: None,
            path: Some(path.to_string_lossy().into_owned()),
            temp_path: None,
            resume: None,
            attempts: 0,
            retry_at: None,
            position: id,
            created_at: 0,
            started_at: None,
            finished_at: None,
            expected_hash: expected_hash.map(str::to_owned),
            checksum_algo: None,
            checksum_status: None,
            checksum_retries: 0,
            media: None,
        }
    }

    #[test]
    fn finds_targets_of_entries() {
        let dir = Path::new("/dl/pkg");
        let a = dir.join("Movie.part1.rar");
        let b = dir.join("sub").join("notes.txt");
        let c = dir.join("other").join("notes.txt");
        let files = [(1, a.as_path()), (2, b.as_path())];
        assert_eq!(find_target(&files, dir, "movie.PART1.rar"), Some(1));
        assert_eq!(find_target(&files, dir, "sub/notes.txt"), Some(2));
        // No folder in the entry: the only file of that name.
        assert_eq!(find_target(&files, dir, "notes.txt"), Some(2));
        let ambiguous = [(2, b.as_path()), (3, c.as_path())];
        assert_eq!(find_target(&ambiguous, dir, "notes.txt"), None);
        assert_eq!(find_target(&files, dir, "elsewhere/notes.txt"), None);
        assert_eq!(find_target(&files, dir, "missing.rar"), None);
    }

    #[test]
    fn detects_work() {
        let dir = Path::new("/dl/pkg");
        let plain = file(1, &dir.join("a.mkv"), None);
        assert!(!has_work(std::slice::from_ref(&plain)));
        assert!(has_work(&[plain.clone(), file(2, &dir.join("a.SFV"), None)]));

        let mut hashed = file(3, &dir.join("b.mkv"), Some("md5:5d41402abc4b2a76b9719d911017c592"));
        assert!(has_work(std::slice::from_ref(&hashed)));
        hashed.checksum_status = Some(OK.into());
        assert!(!has_work(std::slice::from_ref(&hashed)));
        hashed.status = DownloadStatus::Skipped;
        hashed.checksum_status = None;
        assert!(!has_work(&[hashed]));
    }

    #[tokio::test]
    async fn plans_the_strongest_digest_per_file() {
        let dir = tempfile::tempdir().unwrap();
        let sfv = dir.path().join("set.sfv");
        let md5 = dir.path().join("b.bin.md5");
        std::fs::write(&sfv, "a.bin 3610A686\nb.bin 0D4A1185\nnot-in-package.bin 00000000\n").unwrap();
        std::fs::write(&md5, "5d41402abc4b2a76b9719d911017c592\n").unwrap();
        let mut verified = file(4, &dir.path().join("c.bin"), Some("md5:5d41402abc4b2a76b9719d911017c592"));
        verified.checksum_status = Some(OK.into());
        let files = [
            file(1, &dir.path().join("a.bin"), None),
            file(2, &dir.path().join("b.bin"), None),
            file(3, &sfv, None),
            verified,
            file(5, &md5, None),
        ];
        let mut checks = plan(&files).await;
        checks.sort_by_key(|c| c.id);
        let planned: Vec<(i64, String)> = checks.iter().map(|c| (c.id, c.hash.to_string())).collect();
        assert_eq!(planned, [(1, "crc32:3610a686".to_owned()), (2, "md5:5d41402abc4b2a76b9719d911017c592".to_owned())]);
        assert_eq!(checks[0].name, "a.bin");
    }

    const HELLO_MD5: &str = "md5:5d41402abc4b2a76b9719d911017c592";

    async fn test_engine(dir: &Path) -> Arc<Engine> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let config = crate::config::Config::for_tests(dir);
        let db = db::open(&config.database_path()).await.unwrap();
        let settings = Arc::new(crate::settings::SettingsStore::load(db.clone()).await.unwrap());
        let events = Arc::new(crate::events::EventBus::new());
        Engine::new(db, Arc::new(config), settings, events, tokio_util::sync::CancellationToken::new()).await.unwrap()
    }

    /// A package in the verify stage whose files are completed with the given content.
    async fn verifying_package(engine: &Engine, dir: &Path, files: &[(&str, &str, Option<&str>)]) -> i64 {
        let package = db::packages::NewPackage {
            name: "Package".into(),
            name_locked: true,
            category_id: None,
            create_subfolder: false,
            extract: false,
            delete_archives: false,
            archive_password: None,
            source_kind: crate::models::SourceKind::Links,
            source: None,
            stage: Stage::Verify,
            remote_account_id: None,
            remote_id: None,
            paused: false,
        };
        let id = db::packages::insert(&engine.db, &package).await.unwrap();
        let new: Vec<db::downloads::NewDownload> = files
            .iter()
            .map(|(name, _, _)| db::downloads::NewDownload {
                url: format!("https://example.com/{name}"),
                rel_dir: String::new(),
                filename: Some((*name).to_owned()),
                size: None,
                host: None,
                account_id: None,
            })
            .collect();
        db::downloads::insert_many(&engine.db, id, &new).await.unwrap();
        let rows = db::downloads::list_by_package(&engine.db, id).await.unwrap();
        for (row, (name, content, hash)) in rows.iter().zip(files) {
            complete(engine, row.id, &dir.join(name), content).await;
            let update = DownloadUpdate { expected_hash: Some(hash.map(str::to_owned)), ..Default::default() };
            db::downloads::update(&engine.db, row.id, update).await.unwrap();
        }
        id
    }

    /// What a finished download leaves behind.
    async fn complete(engine: &Engine, id: i64, path: &Path, content: &str) {
        std::fs::write(path, content).unwrap();
        let update = DownloadUpdate {
            status: Some(DownloadStatus::Completed),
            path: Some(Some(path.to_string_lossy().into_owned())),
            checksum_algo: Some(None),
            checksum_status: Some(None),
            ..Default::default()
        };
        db::downloads::update(&engine.db, id, update).await.unwrap();
    }

    async fn verify(engine: &Arc<Engine>, package_id: i64) {
        let verification = engine.live.start_verification(package_id, engine.shutdown_token());
        run(engine.clone(), package_id, verification).await;
    }

    async fn file_named(engine: &Engine, package_id: i64, name: &str) -> DownloadRow {
        let files = db::downloads::list_by_package(&engine.db, package_id).await.unwrap();
        files.into_iter().find(|f| f.filename.as_deref() == Some(name)).unwrap()
    }

    #[tokio::test]
    async fn verifies_downloads_again_then_fails_corrupted_files() {
        let dir = tempfile::tempdir().unwrap();
        let engine = test_engine(dir.path()).await;
        let files = dir.path().join("files");
        std::fs::create_dir_all(&files).unwrap();
        let package_id = verifying_package(
            &engine,
            &files,
            &[
                ("good.bin", "hello", None),
                ("bad.bin", "corrupted", Some(HELLO_MD5)),
                ("set.sfv", "good.bin 3610A686", None),
            ],
        )
        .await;

        // First mismatch: the file is deleted and downloaded again.
        verify(&engine, package_id).await;
        let good = file_named(&engine, package_id, "good.bin").await;
        assert_eq!((good.checksum_algo.as_deref(), good.checksum_status.as_deref()), (Some("crc32"), Some(OK)));
        let bad = file_named(&engine, package_id, "bad.bin").await;
        assert_eq!(bad.status, DownloadStatus::Queued);
        assert_eq!((bad.checksum_algo.as_deref(), bad.checksum_status.as_deref()), (Some("md5"), Some(MISMATCH)));
        assert_eq!((bad.checksum_retries, bad.path), (1, None));
        assert!(!files.join("bad.bin").exists());
        let package = db::packages::get(&engine.db, package_id).await.unwrap().unwrap();
        assert_eq!(package.stage, Stage::Download);

        // Still corrupted after the new download: the file fails.
        complete(&engine, bad.id, &files.join("bad.bin"), "corrupted again").await;
        assert!(db::packages::advance(&engine.db, package_id, Stage::Download, Stage::Verify).await.unwrap());
        verify(&engine, package_id).await;
        let bad = file_named(&engine, package_id, "bad.bin").await;
        assert_eq!(bad.status, DownloadStatus::Failed);
        assert_eq!(bad.error.as_deref(), Some("checksum mismatch (md5)"));
        let package = db::packages::get(&engine.db, package_id).await.unwrap().unwrap();
        let files_now = db::downloads::list_by_package(&engine.db, package_id).await.unwrap();
        assert_eq!(package.status(&files_now), crate::models::PackageStatus::Failed);

        // Retried by the user and correct this time: the package finishes.
        complete(&engine, bad.id, &files.join("bad.bin"), "hello").await;
        assert!(db::packages::advance(&engine.db, package_id, Stage::Download, Stage::Verify).await.unwrap());
        verify(&engine, package_id).await;
        let bad = file_named(&engine, package_id, "bad.bin").await;
        assert_eq!(bad.checksum_status.as_deref(), Some(OK));
        let package = db::packages::get(&engine.db, package_id).await.unwrap().unwrap();
        assert_eq!(package.stage, Stage::Done);
        assert!(package.finished_at.is_some() && package.error.is_none());
        assert!(!engine.live.is_verifying(package_id));
    }

    #[tokio::test]
    async fn skips_verification_when_disabled() {
        let dir = tempfile::tempdir().unwrap();
        let engine = test_engine(dir.path()).await;
        engine.settings.modify(|s| s.postprocess.verify_checksums = false).await.unwrap();
        let package_id = verifying_package(&engine, dir.path(), &[("bad.bin", "corrupted", Some(HELLO_MD5))]).await;
        verify(&engine, package_id).await;
        let bad = file_named(&engine, package_id, "bad.bin").await;
        assert_eq!((bad.status, bad.checksum_status), (DownloadStatus::Completed, None));
        let package = db::packages::get(&engine.db, package_id).await.unwrap().unwrap();
        assert_eq!(package.stage, Stage::Done);
    }

    async fn stats(engine: &Engine) -> crate::stats::Totals {
        crate::stats::report(&engine.db, 7, chrono::Local::now().date_naive()).await.unwrap().totals
    }

    #[tokio::test]
    async fn downloads_files_rejected_by_an_interrupted_verification() {
        let dir = tempfile::tempdir().unwrap();
        let engine = test_engine(dir.path()).await;
        let package_id = verifying_package(
            &engine,
            dir.path(),
            &[("good.bin", "hello", Some(HELLO_MD5)), ("bad.bin", "corrupted", Some(HELLO_MD5))],
        )
        .await;
        // A previous run deleted and queued `bad.bin`, then stopped before moving the package.
        let bad = file_named(&engine, package_id, "bad.bin").await;
        std::fs::remove_file(dir.path().join("bad.bin")).unwrap();
        let update = DownloadUpdate { status: Some(DownloadStatus::Queued), path: Some(None), ..Default::default() };
        db::downloads::update(&engine.db, bad.id, update).await.unwrap();

        verify(&engine, package_id).await;
        let package = db::packages::get(&engine.db, package_id).await.unwrap().unwrap();
        assert_eq!((package.stage, package.finished_at), (Stage::Download, None));
        assert_eq!(file_named(&engine, package_id, "bad.bin").await.status, DownloadStatus::Queued);
        assert_eq!(file_named(&engine, package_id, "good.bin").await.checksum_status.as_deref(), Some(OK));
    }

    #[tokio::test]
    async fn fails_files_that_cannot_be_read() {
        let dir = tempfile::tempdir().unwrap();
        let engine = test_engine(dir.path()).await;
        let package_id = verifying_package(
            &engine,
            dir.path(),
            &[("good.bin", "hello", Some(HELLO_MD5)), ("gone.bin", "hello", Some(HELLO_MD5))],
        )
        .await;
        std::fs::remove_file(dir.path().join("gone.bin")).unwrap();

        verify(&engine, package_id).await;
        let gone = file_named(&engine, package_id, "gone.bin").await;
        assert_eq!(gone.status, DownloadStatus::Failed);
        assert!(gone.error.as_deref().is_some_and(|e| e.starts_with("cannot read the file to verify its checksum")));
        let package = db::packages::get(&engine.db, package_id).await.unwrap().unwrap();
        let files = db::downloads::list_by_package(&engine.db, package_id).await.unwrap();
        assert_eq!((package.stage, package.status(&files)), (Stage::Download, crate::models::PackageStatus::Failed));
        assert_eq!(stats(&engine).await.failed, 1);
    }

    #[tokio::test]
    async fn counts_each_failed_file() {
        let dir = tempfile::tempdir().unwrap();
        let engine = test_engine(dir.path()).await;
        let package_id = verifying_package(
            &engine,
            dir.path(),
            &[("a.bin", "corrupted", Some(HELLO_MD5)), ("b.bin", "corrupted", Some(HELLO_MD5))],
        )
        .await;
        // Both were downloaded again already: they fail now.
        for name in ["a.bin", "b.bin"] {
            let file = file_named(&engine, package_id, name).await;
            let update = DownloadUpdate { checksum_retries: Some(MAX_RETRIES), ..Default::default() };
            db::downloads::update(&engine.db, file.id, update).await.unwrap();
        }
        verify(&engine, package_id).await;
        assert_eq!(stats(&engine).await.failed, 2);
    }

    #[tokio::test]
    async fn counts_a_package_extracted_again_once() {
        let dir = tempfile::tempdir().unwrap();
        let engine = test_engine(dir.path()).await;
        let package_id = verifying_package(&engine, dir.path(), &[("a.bin", "hello", Some(HELLO_MD5))]).await;
        verify(&engine, package_id).await;
        assert_eq!(stats(&engine).await.packages, 1);

        assert!(engine.extract_package(package_id).await.unwrap());
        let package = db::packages::get(&engine.db, package_id).await.unwrap().unwrap();
        lifecycle::finish(&engine, &package, Stage::Extract, None).await.unwrap();
        let package = db::packages::get(&engine.db, package_id).await.unwrap().unwrap();
        assert_eq!(package.stage, Stage::Done);
        assert_eq!(stats(&engine).await.packages, 1);
    }
}
