//! User scripts run on package events, with details passed as environment variables:
//! `RUSTLOAD_EVENT`, `RUSTLOAD_PACKAGE_ID`, `RUSTLOAD_PACKAGE_NAME`, `RUSTLOAD_PACKAGE_DIR`,
//! `RUSTLOAD_PACKAGE_STATUS`, `RUSTLOAD_PACKAGE_SIZE`, `RUSTLOAD_FILES` (one path per line).

use std::{process::Stdio, time::Duration};

use tokio::process::Command;

use crate::{
    models::{DownloadRow, PackageRow},
    settings::NotificationEvent,
};

const TIMEOUT: Duration = Duration::from_secs(30 * 60);

pub struct PackageInfo {
    event: &'static str,
    id: i64,
    name: String,
    dir: String,
    status: &'static str,
    size: i64,
    files: Vec<String>,
}

impl PackageInfo {
    pub fn new(package: &PackageRow, files: &[DownloadRow], event: NotificationEvent) -> Self {
        let (event, status) = match event {
            NotificationEvent::PackageFinished => ("package_finished", "completed"),
            NotificationEvent::ExtractFailed => ("extract_failed", "failed"),
            _ => ("package_failed", "failed"),
        };
        Self {
            event,
            id: package.id,
            name: package.name.clone(),
            dir: package.dest_dir.clone().unwrap_or_default(),
            status,
            size: files.iter().filter_map(|f| f.size).sum(),
            files: files.iter().filter_map(|f| f.path.clone()).collect(),
        }
    }
}

/// Runs `command` through the system shell in the background.
pub fn spawn(command: String, info: PackageInfo) {
    tokio::spawn(async move {
        if let Err(error) = run(&command, &info).await {
            tracing::warn!("hook `{command}` failed: {error}");
        }
    });
}

async fn run(command: &str, info: &PackageInfo) -> Result<(), String> {
    let mut process = if cfg!(windows) {
        let mut cmd = Command::new("cmd");
        cmd.arg("/C").arg(command);
        cmd
    } else {
        let mut cmd = Command::new("sh");
        cmd.arg("-c").arg(command);
        cmd
    };
    process
        .env("RUSTLOAD_EVENT", info.event)
        .env("RUSTLOAD_PACKAGE_ID", info.id.to_string())
        .env("RUSTLOAD_PACKAGE_NAME", &info.name)
        .env("RUSTLOAD_PACKAGE_DIR", &info.dir)
        .env("RUSTLOAD_PACKAGE_STATUS", info.status)
        .env("RUSTLOAD_PACKAGE_SIZE", info.size.to_string())
        .env("RUSTLOAD_FILES", info.files.join("\n"))
        .stdin(Stdio::null())
        .kill_on_drop(true);

    tracing::info!("running hook for `{}`", info.name);
    let output = tokio::time::timeout(TIMEOUT, process.output())
        .await
        .map_err(|_| "timed out after 30 minutes".to_owned())?
        .map_err(|e| e.to_string())?;
    for line in
        String::from_utf8_lossy(&output.stdout).lines().chain(String::from_utf8_lossy(&output.stderr).lines()).take(50)
    {
        tracing::info!("hook: {line}");
    }
    if output.status.success() { Ok(()) } else { Err(format!("exit status {}", output.status)) }
}
