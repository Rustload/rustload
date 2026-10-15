//! Package-level transitions: all files done → checksum verification → extraction → finished
//! (history), or failed.
//!
//! Stage changes are conditional updates (`... WHERE stage = <expected>`): when two files of a
//! package settle at the same time, only one caller wins the transition and sends notifications.

use anyhow::Result;

use super::{Engine, remote, verify};
use crate::{
    db,
    events::{Notice, NoticeLevel},
    extract,
    integrations::{hooks, notify::Message},
    models::{DownloadRow, DownloadStatus, PackageRow, Stage},
    settings::NotificationEvent,
};

/// What happened to the file that triggered [`on_file_settled`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Settled {
    /// The file failed for good: count it, and report it if the package cannot progress anymore.
    Failed,
    /// Checksum verification failed files (counted as they were rejected): report them like
    /// [`Self::Failed`].
    Rejected,
    /// Completed, skipped or removed.
    Other,
}

/// Called whenever a file reaches a final state (completed, skipped, failed) or is removed.
pub async fn on_file_settled(engine: &Engine, package_id: i64, cause: Settled) -> Result<()> {
    if cause == Settled::Failed {
        engine.stats.file_failed().await;
    }
    let Some(package) = db::packages::get(&engine.db, package_id).await? else { return Ok(()) };
    if package.stage != Stage::Download {
        return Ok(());
    }
    let files = db::downloads::list_by_package(&engine.db, package_id).await?;
    if files.is_empty() {
        return Ok(());
    }

    if files.iter().all(|f| f.status.is_done()) {
        if engine.settings.get().postprocess.verify_checksums && verify::has_work(&files) {
            if db::packages::advance(&engine.db, package_id, Stage::Download, Stage::Verify).await? {
                engine.events.queue_changed();
                engine.wake();
            }
        } else {
            after_download(engine, &package, &files, Stage::Download).await?;
        }
        return Ok(());
    }

    let pending = files.iter().any(|f| f.status != DownloadStatus::Failed && !f.status.is_done());
    if !pending && cause != Settled::Other {
        // Nothing left to try: the package stays in the queue, failed, until the user acts.
        let reason = files
            .iter()
            .filter(|f| f.status == DownloadStatus::Failed)
            .find_map(|f| f.error.as_deref())
            .unwrap_or("download failed")
            .to_owned();
        report_failure(engine, &package, &reason, NotificationEvent::PackageFailed).await;
    }
    Ok(())
}

/// Every file is downloaded (and verified): extracts the archives, or finishes the package.
pub async fn after_download(engine: &Engine, package: &PackageRow, files: &[DownloadRow], from: Stage) -> Result<()> {
    let paths: Vec<_> = files.iter().filter_map(DownloadRow::completed_path).collect();
    if package.extract && !extract::find_archives(&paths).is_empty() {
        if db::packages::advance(&engine.db, package.id, from, Stage::Extract).await? {
            engine.events.queue_changed();
            engine.wake();
        }
        Ok(())
    } else {
        finish(engine, package, from, None).await
    }
}

/// Moves a package from `from` to the history. `error` marks it as failed (e.g. extraction error).
pub async fn finish(engine: &Engine, package: &PackageRow, from: Stage, error: Option<String>) -> Result<()> {
    if !db::packages::complete(&engine.db, package.id, from, error.as_deref()).await? {
        return Ok(());
    }
    engine.events.queue_changed();
    engine.events.history_changed();

    if engine.settings.get().debrid.delete_magnets {
        remote::forget(engine, package).await;
    }

    match &error {
        None => {
            tracing::info!("package `{}` completed", package.name);
            engine.stats.package_finished(package.id).await;
            engine.events.notice(
                Notice::new(NoticeLevel::Success, "package_finished", format!("{} completed", package.name))
                    .param("name", &package.name),
            );
            dispatch(engine, package, NotificationEvent::PackageFinished, None).await;
        }
        Some(reason) => {
            tracing::warn!("package `{}` finished with an error: {reason}", package.name);
            engine.events.notice(
                Notice::new(NoticeLevel::Warning, "extract_failed", format!("{}: {reason}", package.name))
                    .param("name", &package.name)
                    .param("error", reason),
            );
            dispatch(engine, package, NotificationEvent::ExtractFailed, Some(reason)).await;
        }
    }
    check_queue_finished(engine).await;
    Ok(())
}

/// Notifies a package failure (debrid error, files failed for good).
pub async fn report_failure(engine: &Engine, package: &PackageRow, reason: &str, event: NotificationEvent) {
    tracing::error!("package `{}` failed: {reason}", package.name);
    engine.events.notice(
        Notice::new(NoticeLevel::Error, "package_failed", format!("{} failed: {reason}", package.name))
            .param("name", &package.name)
            .param("error", reason),
    );
    dispatch(engine, package, event, Some(reason)).await;
}

/// Sends notifications and runs the user hook for a package event.
async fn dispatch(engine: &Engine, package: &PackageRow, event: NotificationEvent, error: Option<&str>) {
    let files = db::downloads::list_by_package(&engine.db, package.id).await.unwrap_or_default();
    let size: i64 = files.iter().filter_map(|f| f.size).sum();
    let message = Message::for_package(event, &package.name, size.max(0) as u64, error);
    let settings = engine.settings.get();
    engine.notifier.send(&settings.notifications.targets, event, message);

    let command = match event {
        NotificationEvent::PackageFinished => &settings.hooks.package_finished,
        NotificationEvent::PackageFailed | NotificationEvent::ExtractFailed => &settings.hooks.package_failed,
        NotificationEvent::QueueFinished | NotificationEvent::ChecksumFailed | NotificationEvent::DiskLow => return,
    };
    if !command.is_empty() {
        hooks::spawn(command.clone(), hooks::PackageInfo::new(package, &files, event));
    }
}

/// Fires `queue_finished` when nothing is left to do automatically (paused and failed packages
/// wait for the user and do not count).
async fn check_queue_finished(engine: &Engine) {
    let Ok(queue) = db::packages::list_queue(&engine.db).await else { return };
    let ids: Vec<i64> = queue.iter().map(|p| p.id).collect();
    let Ok(files) = db::downloads::list_by_packages(&engine.db, &ids).await else { return };
    let pending = queue.iter().filter(|p| !p.paused).any(|p| match p.stage {
        Stage::Remote => p.error.is_none(),
        Stage::Extract | Stage::Verify => true,
        Stage::Done => false,
        Stage::Download => files.get(&p.id).is_some_and(|f| {
            f.iter().any(|f| {
                matches!(f.status, DownloadStatus::Queued | DownloadStatus::Resolving | DownloadStatus::Downloading)
            })
        }),
    });
    if !pending {
        tracing::info!("queue finished");
        engine.events.notice(Notice::new(NoticeLevel::Info, "queue_finished", "All downloads are finished"));
        let settings = engine.settings.get();
        engine.notifier.send(
            &settings.notifications.targets,
            NotificationEvent::QueueFinished,
            Message::queue_finished(),
        );
    }
}
