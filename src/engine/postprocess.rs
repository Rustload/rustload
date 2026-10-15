//! Extraction stage of a package.

use std::{
    path::PathBuf,
    sync::{Arc, atomic::Ordering},
};

use super::{Engine, lifecycle, live::Extraction};
use crate::{
    db,
    extract::{ExtractError, find_archives},
    models::{DownloadRow, Stage},
};

pub async fn run(engine: Arc<Engine>, package_id: i64, extraction: Extraction) {
    let result = extract_package(&engine, package_id, &extraction).await;
    let outcome = match db::packages::get(&engine.db, package_id).await {
        Ok(Some(package)) => match result {
            Ok(()) => lifecycle::finish(&engine, &package, Stage::Extract, None).await,
            // Shutdown: stay in the extract stage, it restarts on next boot.
            Err(error) if extraction.cancel.is_cancelled() => {
                tracing::info!("extraction of `{}` interrupted: {error}", package.name);
                Ok(())
            }
            Err(error) => lifecycle::finish(&engine, &package, Stage::Extract, Some(error)).await,
        },
        // Deleted while extracting.
        Ok(None) => Ok(()),
        Err(error) => Err(error),
    };
    if let Err(error) = outcome {
        tracing::error!("cannot finish package {package_id}: {error:#}");
    }
    // Released only now: the scheduler must not see the package still in the extract stage
    // without a running extraction, or it would start a second one.
    engine.live.finish_extraction(package_id);
    engine.wake();
}

async fn extract_package(engine: &Engine, package_id: i64, extraction: &Extraction) -> Result<(), String> {
    let package =
        db::packages::get(&engine.db, package_id).await.map_err(|e| e.to_string())?.ok_or("package not found")?;
    let files = db::downloads::list_by_package(&engine.db, package_id).await.map_err(|e| e.to_string())?;
    let paths: Vec<PathBuf> = files.iter().filter_map(DownloadRow::completed_path).filter(|p| p.exists()).collect();
    let sets = find_archives(&paths);
    if sets.is_empty() {
        return Ok(());
    }

    let settings = engine.settings.get();
    let mut passwords: Vec<String> = package.archive_password.iter().cloned().collect();
    for password in &settings.postprocess.passwords {
        if !passwords.contains(password) {
            passwords.push(password.clone());
        }
    }

    let count = sets.len();
    for (index, set) in sets.iter().enumerate() {
        let name = set.first.file_name().unwrap_or_default().to_string_lossy().into_owned();
        tracing::info!("extracting {name}");
        let shared = extraction.progress.clone();
        let report: crate::extract::Progress = Arc::new(move |fraction: f64| {
            let overall = (index as f64 + fraction) / count as f64;
            shared.store((overall * 1000.0) as u32, Ordering::Relaxed);
        });
        let extracted = match engine.extractor.extract(set, &passwords, report, &extraction.cancel).await {
            Ok(extracted) => extracted,
            // A `.zip` that is really something else: keep it as a regular file.
            Err(ExtractError::NotArchive) => {
                tracing::warn!("{name} is not an archive, left as is");
                continue;
            }
            Err(error) => return Err(format!("{name}: {error}")),
        };
        tracing::info!("extracted {name} ({} item(s))", extracted.entries.len());

        if let Some(password) = &extracted.password {
            promote_password(engine, password).await;
        }
        if package.delete_archives {
            for volume in &set.volumes {
                if let Err(error) = tokio::fs::remove_file(volume).await {
                    tracing::warn!("cannot delete {}: {error}", volume.display());
                }
            }
        }
    }
    Ok(())
}

/// Moves a password that worked to the top of the global list, so the
/// next archive of the same source is opened on the first try.
async fn promote_password(engine: &Engine, password: &str) {
    let settings = engine.settings.get();
    let list = &settings.postprocess.passwords;
    if list.first().map(String::as_str) == Some(password) || !list.iter().any(|p| p == password) {
        return;
    }
    let password = password.to_owned();
    let result = engine
        .settings
        .modify(move |s| {
            s.postprocess.passwords.retain(|p| *p != password);
            s.postprocess.passwords.insert(0, password);
        })
        .await;
    if let Err(error) = result {
        tracing::warn!("cannot reorder archive passwords: {error:#}");
    }
}
