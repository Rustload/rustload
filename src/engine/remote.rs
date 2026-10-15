//! Magnets and torrents are downloaded by the debrid service first ("remote" stage); this module
//! polls the provider until the files are ready, then turns them into regular downloads.

use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};

use anyhow::{Context, Result};

use super::{DebridProgress, Engine, intake::Source, lifecycle};
use crate::{
    db::{self, downloads::NewDownload, packages::PackageUpdate},
    debrid::{AccountHandle, DebridError, RemoteFile, TransferState},
    events::{Notice, NoticeLevel},
    models::{PackageRow, Stage},
    settings::NotificationEvent,
};

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const SLOW_POLL_INTERVAL: Duration = Duration::from_secs(15);

pub async fn run(engine: Arc<Engine>, package_id: i64) {
    if let Err(error) = poll(&engine, package_id).await {
        tracing::error!("debrid polling of package {package_id} stopped: {error:#}");
    }
    engine.remote_done(package_id);
    engine.events.queue_changed();
    engine.wake();
}

async fn poll(engine: &Engine, package_id: i64) -> Result<()> {
    let mut last_saved: Option<DebridProgress> = None;
    loop {
        let Some(package) = db::packages::get(&engine.db, package_id).await? else { return Ok(()) };
        if package.stage != Stage::Remote || package.error.is_some() {
            return Ok(());
        }
        let (Some(account_id), Some(remote_id)) = (package.remote_account_id, package.remote_id.clone()) else {
            return fail(engine, &package, "the debrid transfer is missing").await;
        };
        let Some(account) = engine.debrid.get(account_id).await else {
            return fail(engine, &package, "the debrid account holding this transfer is disabled or deleted").await;
        };

        let status = match account.transfer_status(&remote_id).await {
            Ok(status) => status,
            Err(DebridError::Temporary(message)) => {
                tracing::debug!("debrid status of `{}` unavailable: {message}", package.name);
                sleep_or_shutdown(engine, SLOW_POLL_INTERVAL).await?;
                continue;
            }
            Err(error) => return fail(engine, &package, &error.to_string()).await,
        };

        let progress = DebridProgress {
            status: status.status_text.clone(),
            progress: status.progress,
            speed: status.speed,
            seeders: status.seeders,
            size: status.size,
        };
        engine.set_remote_progress(package_id, progress.clone());
        if last_saved.as_ref() != Some(&progress) {
            let state = serde_json::to_string(&progress).ok();
            db::packages::update(
                &engine.db,
                package_id,
                PackageUpdate { remote_state: Some(state), ..Default::default() },
            )
            .await?;
            last_saved = Some(progress);
        }

        match status.state {
            TransferState::Ready => {
                let files = match account.transfer_files(&remote_id).await {
                    Ok(files) if !files.is_empty() => files,
                    Ok(_) => return fail(engine, &package, "the debrid service returned no files").await,
                    Err(DebridError::Temporary(_)) => {
                        sleep_or_shutdown(engine, SLOW_POLL_INTERVAL).await?;
                        continue;
                    }
                    Err(error) => return fail(engine, &package, &error.to_string()).await,
                };
                return materialize(engine, &package, &account, status.name, files).await;
            }
            TransferState::Error => return fail(engine, &package, &status.status_text).await,
            TransferState::Downloading => sleep_or_shutdown(engine, SLOW_POLL_INTERVAL).await?,
            _ => sleep_or_shutdown(engine, POLL_INTERVAL).await?,
        }
    }
}

/// Creates one download per remote file and moves the package to the download stage.
async fn materialize(
    engine: &Engine,
    package: &PackageRow,
    account: &AccountHandle,
    remote_name: Option<String>,
    files: Vec<RemoteFile>,
) -> Result<()> {
    let name = match remote_name {
        Some(name) if !package.name_locked && !name.trim().is_empty() => name,
        _ => package.name.clone(),
    };
    // A package folder named after the torrent already plays the role of its root folder.
    let strip_root = package.create_subfolder.then(|| common_root(&files)).flatten();
    let downloads: Vec<NewDownload> = files
        .iter()
        .map(|file| {
            let relative = match &strip_root {
                Some(root) => file.path.strip_prefix(root.as_str()).unwrap_or(&file.path).trim_start_matches('/'),
                None => file.path.as_str(),
            };
            let (dir, file_name) = match relative.rsplit_once('/') {
                Some((dir, file_name)) => (dir.to_owned(), file_name.to_owned()),
                None => (String::new(), relative.to_owned()),
            };
            NewDownload {
                url: file.link.clone(),
                rel_dir: dir,
                filename: Some(file_name),
                size: file.size.map(|s| s as i64),
                host: Some(account.provider.to_owned()),
                account_id: Some(account.id),
                media: None,
            }
        })
        .collect();

    // After a crash between both writes, the files exist already: do not add them twice.
    if db::downloads::count_by_package(&engine.db, package.id).await? == 0 {
        db::downloads::insert_many(&engine.db, package.id, &downloads).await?;
    }
    db::packages::update(
        &engine.db,
        package.id,
        PackageUpdate { name: Some(name.clone()), stage: Some(Stage::Download), ..Default::default() },
    )
    .await?;
    if package.paused {
        db::downloads::set_status_where(
            &engine.db,
            package.id,
            &[crate::models::DownloadStatus::Queued],
            crate::models::DownloadStatus::Paused,
        )
        .await?;
    }
    tracing::info!("`{name}` is ready on {} ({} file(s))", account.label, downloads.len());
    engine.events.notice(
        Notice::new(NoticeLevel::Info, "debrid_ready", format!("{name} is ready on the debrid service"))
            .param("name", &name),
    );
    engine.events.queue_changed();
    engine.wake();
    Ok(())
}

/// `Show/` when every file lives under the same top-level folder.
fn common_root(files: &[RemoteFile]) -> Option<String> {
    let first = files.first()?.path.split_once('/')?.0;
    files.iter().all(|f| f.path.split_once('/').is_some_and(|(root, _)| root == first)).then(|| format!("{first}/"))
}

async fn fail(engine: &Engine, package: &PackageRow, reason: &str) -> Result<()> {
    db::packages::update(
        &engine.db,
        package.id,
        PackageUpdate { error: Some(Some(reason.to_owned())), ..Default::default() },
    )
    .await?;
    lifecycle::report_failure(engine, package, reason, NotificationEvent::PackageFailed).await;
    engine.events.queue_changed();
    Ok(())
}

async fn sleep_or_shutdown(engine: &Engine, duration: Duration) -> Result<()> {
    tokio::select! {
        _ = engine.shutdown_token().cancelled() => anyhow::bail!("shutting down"),
        _ = tokio::time::sleep(duration) => Ok(()),
    }
}

/// Where the original `.torrent` of a package is kept, to submit it again on retry.
pub fn torrent_path(config_dir: &Path, package_id: i64) -> PathBuf {
    config_dir.join("torrents").join(format!("{package_id}.torrent"))
}

/// Submits the magnet/torrent again after a remote failure.
pub async fn restart(engine: &Engine, package: &PackageRow) -> Result<()> {
    let account = engine.debrid.magnet_account().await.context("no enabled debrid account supports magnets")?;
    let remote_id = match Source::of(engine, package).await? {
        Source::Magnet(magnet) => account.add_magnet(&magnet).await?,
        Source::Torrent { file_name, data } => account.add_torrent(&file_name, data).await?,
        Source::Links(_) => return Ok(()),
    };
    forget(engine, package).await;
    let update = PackageUpdate {
        remote_account_id: Some(Some(account.id)),
        remote_id: Some(Some(remote_id)),
        remote_state: Some(None),
        error: Some(None),
        ..Default::default()
    };
    db::packages::update(&engine.db, package.id, update).await
}

/// Deletes the transfer on the provider side (best effort).
pub async fn forget(engine: &Engine, package: &PackageRow) {
    let (Some(account_id), Some(remote_id)) = (package.remote_account_id, package.remote_id.as_deref()) else {
        return;
    };
    if let Some(account) = engine.debrid.get(account_id).await
        && let Err(error) = account.delete_transfer(remote_id).await
    {
        tracing::debug!("cannot delete debrid transfer {remote_id}: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(path: &str) -> RemoteFile {
        RemoteFile { path: path.into(), size: None, link: String::new() }
    }

    #[test]
    fn detects_common_root_folder() {
        assert_eq!(common_root(&[file("Show/a.mkv"), file("Show/sub/b.srt")]).as_deref(), Some("Show/"));
        assert_eq!(common_root(&[file("Show/a.mkv"), file("Other/b.mkv")]), None);
        assert_eq!(common_root(&[file("single.mkv")]), None);
    }
}
