//! 7-Zip command line driver (RAR, 7z, ZIP, tar, split files).

use std::{
    path::{Path, PathBuf},
    process::Stdio,
};

use tokio::{io::AsyncReadExt, process::Command};
use tokio_util::sync::CancellationToken;

use super::{ExtractError, Progress};
use crate::util::NoWindow;

const CANDIDATES: [&str; 3] = ["7zz", "7z", "7za"];

/// Looks for a 7-Zip binary in `bin_dir` (installed by Rustload), then in `PATH` (and Homebrew's
/// folders on macOS) and the default install folder on Windows.
pub fn find(bin_dir: &Path) -> Option<PathBuf> {
    let mut dirs = vec![bin_dir.to_path_buf()];
    dirs.extend(crate::util::tool_dirs());
    if cfg!(windows) {
        dirs.push(PathBuf::from(r"C:\Program Files\7-Zip"));
    }
    for name in CANDIDATES {
        let file = if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() };
        if let Some(found) = dirs.iter().map(|dir| dir.join(&file)).find(|p| p.is_file()) {
            return Some(found);
        }
    }
    None
}

pub async fn extract(
    tool: &Path,
    archive: &Path,
    dest: &Path,
    password: Option<&str>,
    progress: Progress,
    cancel: &CancellationToken,
) -> Result<(), ExtractError> {
    run(tool, archive, dest, password, progress.clone(), cancel).await?;

    // `x.tar.gz` unpacks to `x.tar` with 7-Zip: unpack the inner tarball too.
    if is_compressed_tarball(archive)
        && let Some(inner) = single_tar(dest)?
    {
        run(tool, &inner, dest, None, progress, cancel).await?;
        tokio::fs::remove_file(&inner).await?;
    }
    Ok(())
}

async fn run(
    tool: &Path,
    archive: &Path,
    dest: &Path,
    password: Option<&str>,
    progress: Progress,
    cancel: &CancellationToken,
) -> Result<(), ExtractError> {
    let mut child = Command::new(tool)
        .no_window()
        .arg("x")
        .arg("-y")
        .arg("-aoa")
        .arg("-bso0")
        .arg("-bsp1")
        .arg("-sccUTF-8")
        // "-" is never a real password: 7-Zip fails fast with "Wrong password" instead of asking.
        .arg(format!("-p{}", password.unwrap_or("-")))
        .arg(format!("-o{}", dest.display()))
        .arg("--")
        .arg(archive)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| ExtractError::Failed(format!("cannot start {}: {e}", tool.display())))?;

    let mut stdout = child.stdout.take().expect("piped stdout");
    let mut stderr = child.stderr.take().expect("piped stderr");
    let progress_reader = async move {
        let mut buffer = [0u8; 1024];
        let mut digits = String::new();
        while let Ok(read) = stdout.read(&mut buffer).await {
            if read == 0 {
                break;
            }
            for &byte in &buffer[..read] {
                match byte {
                    b'0'..=b'9' => digits.push(byte as char),
                    b'%' => {
                        if let Ok(percent) = digits.parse::<u32>() {
                            progress(f64::from(percent.min(100)) / 100.0);
                        }
                        digits.clear();
                    }
                    _ => digits.clear(),
                }
            }
        }
    };
    let error_reader = async move {
        let mut output = String::new();
        let _ = stderr.read_to_string(&mut output).await;
        output
    };

    // On cancellation the child is dropped, which kills it (`kill_on_drop`).
    let (status, (), errors) = tokio::select! {
        _ = cancel.cancelled() => return Err(ExtractError::Cancelled),
        outcome = async { tokio::join!(child.wait(), progress_reader, error_reader) } => outcome,
    };
    let status = status?;

    // 0 = success, 1 = warnings only (e.g. a locked file), 2+ = fatal.
    match status.code() {
        Some(0 | 1) => Ok(()),
        _ if errors.to_ascii_lowercase().contains("wrong password") => Err(ExtractError::WrongPassword),
        _ if errors.contains("Is not archive") || errors.contains("Can not open the file as archive") => {
            Err(ExtractError::NotArchive)
        }
        _ if errors.contains("Missing volume") || errors.contains("Unexpected end of archive") => {
            Err(ExtractError::Failed("the archive is incomplete or a volume is missing".into()))
        }
        _ => {
            let message = errors
                .lines()
                .map(str::trim)
                .rfind(|l| !l.is_empty() && !l.starts_with("7-Zip") && !l.starts_with("Scanning"))
                .unwrap_or("extraction failed")
                .to_owned();
            Err(ExtractError::Failed(message))
        }
    }
}

fn is_compressed_tarball(archive: &Path) -> bool {
    let name = archive.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    [".tar.gz", ".tgz", ".tar.bz2", ".tbz", ".tbz2", ".tar.xz", ".txz", ".tar.zst", ".tzst"]
        .iter()
        .any(|ext| name.ends_with(ext))
}

/// The only entry of `dir`, if it is a `.tar` file.
fn single_tar(dir: &Path) -> std::io::Result<Option<PathBuf>> {
    let entries: Vec<PathBuf> = std::fs::read_dir(dir)?.filter_map(Result::ok).map(|e| e.path()).collect();
    Ok(match entries.as_slice() {
        [only] if only.extension().is_some_and(|e| e.eq_ignore_ascii_case("tar")) => Some(only.clone()),
        _ => None,
    })
}
