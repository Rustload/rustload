//! Locating yt-dlp and ffmpeg, installing or updating the managed yt-dlp binary, and installing
//! ffmpeg and 7-Zip on Windows, where no package manager provides them.
//!
//! Lookup order: the path set in the settings, then the managed copy in `<config>/bin/` (installed
//! from the interface and kept up to date), then `PATH`. Detection runs the tools once (`--version`)
//! and is cached, as starting a PyInstaller bundle takes a second.
//!
//! yt-dlp runs with `<config>/tmp` as its temporary folder (see `process::command`), emptied at
//! startup: runs that were killed leave their unpacked build there.

use std::{
    env::consts::{ARCH, OS},
    ffi::OsString,
    fs::File,
    io::BufReader,
    path::{Path, PathBuf},
    sync::{
        Arc, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

use futures_util::StreamExt;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::{io::AsyncWriteExt, sync::Semaphore};
use tokio_util::sync::CancellationToken;

use super::{MediaSettings, process};

const RELEASES_API: &str = "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest";
const CHECKSUMS_ASSET: &str = "SHA2-256SUMS";
/// Detection results are trusted this long (tools rarely appear or vanish while running).
const DETECTION_TTL: Duration = Duration::from_secs(300);
const VERSION_TIMEOUT: Duration = Duration::from_secs(60);
/// Official binaries are about 40 MiB; anything far bigger is not what we expect.
const MAX_BINARY_SIZE: u64 = 300 * 1024 * 1024;
/// First release with `--js-runtimes` (YouTube needs a JavaScript runtime since then).
const JS_RUNTIMES_SINCE: [u64; 3] = [2025, 11, 12];
/// yt-dlp probes running at once (link checks, the link grabber, the API): each is a process
/// using a CPU core and some memory for a few seconds.
const MAX_PROBES: usize = 4;
/// Prefix of a binary being installed in `<config>/bin/`.
const PARTIAL_PREFIX: &str = ".new-";

/// yt-dlp's own FFmpeg builds, the ones it recommends.
const FFMPEG_RELEASES_API: &str = "https://api.github.com/repos/yt-dlp/FFmpeg-Builds/releases/latest";
const FFMPEG_CHECKSUMS_ASSET: &str = "checksums.sha256";
/// The Windows archives weigh about 200 MiB.
const MAX_FFMPEG_ARCHIVE: u64 = 400 * 1024 * 1024;
/// Taken from the archive's `bin` folder; yt-dlp uses both.
const FFMPEG_BINARIES: [&str; 2] = ["ffmpeg.exe", "ffprobe.exe"];

/// 7-Zip's official releases, pinned like the Docker image's.
const SEVEN_ZIP_DOWNLOADS: &str = "https://github.com/ip7z/7zip/releases/download/26.04";
/// The standalone 7z-only tool that unpacks the installer.
const SEVEN_ZIP_REDUCED: (&str, &str) = ("7zr.exe", "256feca8e274e5da655e2a284fabafd9f554365eb164862089dacd4e8276d282");
/// The command line and its library, RAR support included.
const SEVEN_ZIP_BINARIES: [&str; 2] = ["7z.dll", "7z.exe"];

/// A tool found on this machine.
#[derive(Debug, Clone, PartialEq)]
pub struct Tool {
    pub path: PathBuf,
    pub version: Option<String>,
    /// The copy installed and updated by Rustload (yt-dlp only).
    pub managed: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct Tools {
    pub ytdlp: Option<Tool>,
    pub ffmpeg: Option<Tool>,
}

impl Tools {
    /// Whether this yt-dlp knows `--js-runtimes` (YouTube support through Node.js/QuickJS).
    pub fn supports_js_runtimes(&self) -> bool {
        let version = self.ytdlp.as_ref().and_then(|t| t.version.as_deref()).and_then(version_key);
        version.is_some_and(|v| v.as_slice() >= JS_RUNTIMES_SINCE.as_slice())
    }
}

struct Detected {
    at: Instant,
    /// Settings the detection was made with.
    key: (String, String),
    tools: Tools,
}

#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("an installation is already running")]
    Busy,
    #[error("no official {tool} build can be installed on this platform ({platform}); install {tool} yourself")]
    Unsupported { tool: &'static str, platform: String },
    #[error("cannot reach GitHub: {0}")]
    Network(String),
    #[error("{0}")]
    Invalid(String),
    #[error("cannot write {0}: {1}")]
    Io(String, std::io::Error),
}

impl InstallError {
    fn unsupported(tool: &'static str) -> Self {
        Self::Unsupported { tool, platform: format!("{OS}/{ARCH}") }
    }
}

impl From<reqwest::Error> for InstallError {
    fn from(error: reqwest::Error) -> Self {
        Self::Network(error.without_url().to_string())
    }
}

#[derive(Debug, Deserialize)]
pub struct Release {
    pub tag_name: String,
    #[serde(default)]
    assets: Vec<Asset>,
}

#[derive(Debug, Deserialize)]
struct Asset {
    name: String,
    browser_download_url: String,
}

pub struct MediaTools {
    config_dir: PathBuf,
    http: reqwest::Client,
    state: RwLock<Option<Detected>>,
    refresh: tokio::sync::Mutex<()>,
    /// yt-dlp is being installed or updated.
    updating: AtomicBool,
    installing_ffmpeg: AtomicBool,
    installing_seven_zip: AtomicBool,
    /// Latest release seen on GitHub.
    latest: RwLock<Option<String>>,
    probes: Arc<Semaphore>,
}

impl MediaTools {
    /// Also cleans what earlier runs left behind (see [`clean_leftovers`]), so it must be created
    /// before any tool runs.
    pub fn new(config_dir: PathBuf, user_agent: &str) -> reqwest::Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(user_agent)
            .connect_timeout(Duration::from_secs(15))
            .timeout(Duration::from_secs(600))
            .build()?;
        let tools = Self {
            config_dir,
            http,
            state: RwLock::new(None),
            refresh: tokio::sync::Mutex::new(()),
            updating: AtomicBool::new(false),
            installing_ffmpeg: AtomicBool::new(false),
            installing_seven_zip: AtomicBool::new(false),
            latest: RwLock::new(None),
            probes: Arc::new(Semaphore::new(MAX_PROBES)),
        };
        clean_leftovers(&tools.tmp_dir(), &tools.bin_dir());
        Ok(tools)
    }

    pub fn bin_dir(&self) -> PathBuf {
        self.config_dir.join("bin")
    }

    /// Temporary folder of the tools: unpacked yt-dlp builds (Unix), private cookie copies.
    pub fn tmp_dir(&self) -> PathBuf {
        self.config_dir.join("tmp")
    }

    /// Permits for yt-dlp probes, shared by every run (see [`MAX_PROBES`]).
    pub fn probe_slots(&self) -> Arc<Semaphore> {
        self.probes.clone()
    }

    pub fn managed_path(&self) -> PathBuf {
        self.bin_dir().join(executable("yt-dlp"))
    }

    pub fn is_updating(&self) -> bool {
        self.updating.load(Ordering::Acquire)
    }

    pub fn is_installing_ffmpeg(&self) -> bool {
        self.installing_ffmpeg.load(Ordering::Acquire)
    }

    pub fn is_installing_seven_zip(&self) -> bool {
        self.installing_seven_zip.load(Ordering::Acquire)
    }

    pub fn latest_version(&self) -> Option<String> {
        self.latest.read().expect("poisoned").clone()
    }

    /// Last detection result, without detecting again (for cheap status fields).
    pub fn cached(&self) -> Option<Tools> {
        self.state.read().expect("poisoned").as_ref().map(|d| d.tools.clone())
    }

    /// Last detection made with these settings, however old.
    pub fn last(&self, settings: &MediaSettings) -> Option<Tools> {
        let key = (settings.ytdlp_path.clone(), settings.ffmpeg_path.clone());
        let state = self.state.read().expect("poisoned");
        state.as_ref().filter(|d| d.key == key).map(|d| d.tools.clone())
    }

    /// Forgets the detection results, e.g. after an installation.
    pub fn invalidate(&self) {
        *self.state.write().expect("poisoned") = None;
    }

    /// The tools to use with these settings, detected again when the cache is stale.
    pub async fn tools(&self, settings: &MediaSettings) -> Tools {
        let key = (settings.ytdlp_path.clone(), settings.ffmpeg_path.clone());
        if let Some(tools) = self.fresh(&key) {
            return tools;
        }
        let _guard = self.refresh.lock().await;
        if let Some(tools) = self.fresh(&key) {
            return tools;
        }
        let tools = self.detect(settings).await;
        *self.state.write().expect("poisoned") = Some(Detected { at: Instant::now(), key, tools: tools.clone() });
        tools
    }

    fn fresh(&self, key: &(String, String)) -> Option<Tools> {
        let state = self.state.read().expect("poisoned");
        state.as_ref().filter(|d| d.key == *key && d.at.elapsed() < DETECTION_TTL).map(|d| d.tools.clone())
    }

    async fn detect(&self, settings: &MediaSettings) -> Tools {
        let managed = self.managed_path();
        let ytdlp = if !settings.ytdlp_path.is_empty() {
            self.configured(&settings.ytdlp_path, "yt-dlp")
        } else if managed.is_file() {
            Some(managed.clone())
        } else {
            find_in_path("yt-dlp")
        };
        let ffmpeg = if !settings.ffmpeg_path.is_empty() {
            self.configured(&settings.ffmpeg_path, "ffmpeg")
        } else {
            Some(self.bin_dir().join(executable("ffmpeg"))).filter(|p| p.is_file()).or_else(|| find_in_path("ffmpeg"))
        };

        let tmp_dir = self.tmp_dir();
        let ytdlp = match ytdlp {
            Some(path) => match tool_version(&path, "--version", &tmp_dir).await {
                Ok(version) => Some(Tool { managed: path == managed, path, version: Some(version) }),
                Err(error) => {
                    tracing::warn!("yt-dlp at {} does not run: {error}", path.display());
                    None
                }
            },
            None => None,
        };
        let ffmpeg = match ffmpeg {
            Some(path) => match tool_version(&path, "-version", &tmp_dir).await {
                Ok(banner) => Some(Tool { version: ffmpeg_version(&banner), path, managed: false }),
                Err(error) => {
                    tracing::warn!("ffmpeg at {} does not run: {error}", path.display());
                    None
                }
            },
            None => None,
        };
        Tools { ytdlp, ffmpeg }
    }

    /// A binary given in the settings: a file, or the folder containing it; relative paths are
    /// relative to the config folder.
    fn configured(&self, value: &str, name: &str) -> Option<PathBuf> {
        let path = self.config_dir.join(value);
        let path = if path.is_dir() { path.join(executable(name)) } else { path };
        if path.is_file() {
            Some(path)
        } else {
            tracing::warn!("{name} not found at {}", path.display());
            None
        }
    }

    /// Latest official release, remembered for the status.
    pub async fn latest_release(&self) -> Result<Release, InstallError> {
        let release: Release = self.http.get(RELEASES_API).send().await?.error_for_status()?.json().await?;
        *self.latest.write().expect("poisoned") = Some(release.tag_name.clone());
        Ok(release)
    }

    /// Downloads the latest release into `<config>/bin/`. Returns the installed version.
    pub async fn install_latest(&self) -> Result<String, InstallError> {
        let _installing = Installing::start(self, &self.updating)?;
        self.install(&self.latest_release().await?).await
    }

    /// Installs `release` if it is newer than the yt-dlp in use: the managed copy, or the one
    /// bundled with the system or the Docker image (sites change often and old versions break).
    /// A binary set explicitly in the settings is left alone, and nothing is installed when yt-dlp
    /// is absent: that stays the user's choice. Returns the new version.
    pub async fn update_managed(
        &self,
        settings: &MediaSettings,
        release: &Release,
    ) -> Result<Option<String>, InstallError> {
        let tools = self.tools(settings).await;
        let Some(current) = tools.ytdlp else { return Ok(None) };
        if !current.managed && !settings.ytdlp_path.trim().is_empty() {
            return Ok(None);
        }
        if !is_newer(&release.tag_name, current.version.as_deref()) {
            return Ok(None);
        }
        let _installing = Installing::start(self, &self.updating)?;
        self.install(release).await.map(Some)
    }

    async fn install(&self, release: &Release) -> Result<String, InstallError> {
        let name = asset_name(OS, ARCH, is_musl()).ok_or_else(|| InstallError::unsupported("yt-dlp"))?;
        let (url, expected) = self.verified_asset(release, name, CHECKSUMS_ASSET, "yt-dlp").await?;

        let dir = self.bin_dir();
        create_dir(&dir).await?;
        let target = self.managed_path();
        let file_name = target.file_name().unwrap_or_default().to_string_lossy();
        let temp = PartialFile(dir.join(format!("{PARTIAL_PREFIX}{}-{file_name}", &crate::util::random_token()[..8])));
        self.download_verified(&url, &temp.0, &expected, MAX_BINARY_SIZE).await?;
        let version = tool_version(&temp.0, "--version", &self.tmp_dir())
            .await
            .map_err(|e| InstallError::Invalid(format!("the downloaded yt-dlp does not run here: {e}")))?;
        replace(&temp.0, &target).await.map_err(|e| InstallError::Io(target.display().to_string(), e))?;
        tracing::info!("yt-dlp {version} installed in {}", target.display());
        Ok(version)
    }

    /// Downloads yt-dlp's FFmpeg build (Windows only) and installs `ffmpeg.exe` and `ffprobe.exe`
    /// into `<config>/bin/`, where detection looks first. Returns the installed version.
    pub async fn install_ffmpeg(&self) -> Result<String, InstallError> {
        let _installing = Installing::start(self, &self.installing_ffmpeg)?;
        let name = ffmpeg_asset_name(OS, ARCH).ok_or_else(|| InstallError::unsupported("ffmpeg"))?;
        let release: Release = self.http.get(FFMPEG_RELEASES_API).send().await?.error_for_status()?.json().await?;
        let (url, expected) = self.verified_asset(&release, name, FFMPEG_CHECKSUMS_ASSET, "ffmpeg").await?;

        let (tmp_dir, bin_dir) = (self.tmp_dir(), self.bin_dir());
        create_dir(&tmp_dir).await?;
        create_dir(&bin_dir).await?;
        let archive =
            PartialFile(tmp_dir.join(format!("{PARTIAL_PREFIX}{}-{name}", &crate::util::random_token()[..8])));
        self.download_verified(&url, &archive.0, &expected, MAX_FFMPEG_ARCHIVE).await?;
        let (source, dir) = (archive.0.clone(), bin_dir.clone());
        let unpacked = tokio::task::spawn_blocking(move || unpack_ffmpeg(&source, &dir))
            .await
            .map_err(|e| InstallError::Invalid(e.to_string()))??;
        let (ffmpeg, _) = &unpacked[0];
        let banner = tool_version(&ffmpeg.0, "-version", &tmp_dir)
            .await
            .map_err(|e| InstallError::Invalid(format!("the downloaded ffmpeg does not run here: {e}")))?;
        for (partial, target) in &unpacked {
            replace(&partial.0, target).await.map_err(|e| InstallError::Io(target.display().to_string(), e))?;
        }
        let version = ffmpeg_version(&banner).unwrap_or(banner);
        tracing::info!("ffmpeg {version} installed in {}", bin_dir.display());
        Ok(version)
    }

    /// Installs 7-Zip's command line (`7z.exe` and `7z.dll`, RAR support included) into
    /// `<config>/bin/` from its official installer, unpacked by the standalone `7zr.exe` (Windows
    /// only). Returns the installed version.
    pub async fn install_seven_zip(&self) -> Result<String, InstallError> {
        let _installing = Installing::start(self, &self.installing_seven_zip)?;
        let (installer, installer_sum) =
            seven_zip_installer(OS, ARCH).ok_or_else(|| InstallError::unsupported("7-Zip"))?;
        let tmp_dir = self.tmp_dir();
        let work = ScratchDir(tmp_dir.join(format!("7zip-{}", &crate::util::random_token()[..8])));
        create_dir(&work.0).await?;
        let (reduced, reduced_sum) = SEVEN_ZIP_REDUCED;
        for (name, sum) in [(reduced, reduced_sum), (installer, installer_sum)] {
            let url = format!("{SEVEN_ZIP_DOWNLOADS}/{name}");
            self.download_verified(&url, &work.0.join(name), sum, MAX_BINARY_SIZE).await?;
        }

        let out = work.0.join("out");
        let mut args: Vec<OsString> = ["x", "-y"].into_iter().map(OsString::from).collect();
        args.push(format!("-o{}", out.display()).into());
        args.push(work.0.join(installer).into());
        args.extend(SEVEN_ZIP_BINARIES.into_iter().map(OsString::from));
        let unpacked = process::run(&work.0.join(reduced), &args, &tmp_dir, VERSION_TIMEOUT, &CancellationToken::new())
            .await
            .map_err(|e| InstallError::Invalid(format!("cannot unpack the 7-Zip installer: {e}")))?;
        if !unpacked.status.success() {
            let reason = unpacked.error().unwrap_or_else(|| format!("exit status {}", unpacked.status));
            return Err(InstallError::Invalid(format!("cannot unpack the 7-Zip installer: {reason}")));
        }
        let banner = tool_version(&out.join("7z.exe"), "i", &tmp_dir)
            .await
            .map_err(|e| InstallError::Invalid(format!("the downloaded 7-Zip does not run here: {e}")))?;

        let bin_dir = self.bin_dir();
        create_dir(&bin_dir).await?;
        let token = &crate::util::random_token()[..8];
        // The library first: the program needs it. Copied, as `tmp` may be another volume.
        for name in SEVEN_ZIP_BINARIES {
            let (partial, target) =
                (PartialFile(bin_dir.join(format!("{PARTIAL_PREFIX}{token}-{name}"))), bin_dir.join(name));
            let io_error = |e| InstallError::Io(target.display().to_string(), e);
            tokio::fs::copy(out.join(name), &partial.0).await.map_err(io_error)?;
            replace(&partial.0, &target).await.map_err(io_error)?;
        }
        // "7-Zip 26.04 (x64) : Copyright (c) 1999-2026 Igor Pavlov : 2026-10-05"
        let version = banner.split(" : ").next().unwrap_or(&banner).to_owned();
        tracing::info!("{version} installed in {}", bin_dir.display());
        Ok(version)
    }

    /// The download address of asset `name` of `release` and its SHA-256, from the release's
    /// `sums` file: unverified binaries are refused.
    async fn verified_asset(
        &self,
        release: &Release,
        name: &str,
        sums: &str,
        tool: &'static str,
    ) -> Result<(String, String), InstallError> {
        let find = |wanted: &str| release.assets.iter().find(|a| a.name == wanted);
        let asset = find(name).ok_or_else(|| InstallError::unsupported(tool))?;
        let sums = find(sums).ok_or_else(|| {
            InstallError::Invalid("the release has no checksum file: refusing an unverified binary".into())
        })?;
        let sums = self.http.get(&sums.browser_download_url).send().await?.error_for_status()?.text().await?;
        let expected = checksum_for(&sums, name)
            .ok_or_else(|| InstallError::Invalid(format!("no checksum published for {name}")))?;
        Ok((asset.browser_download_url.clone(), expected))
    }

    async fn download_verified(
        &self,
        url: &str,
        path: &Path,
        expected: &str,
        max_size: u64,
    ) -> Result<(), InstallError> {
        let response = self.http.get(url).send().await?.error_for_status()?;
        if response.content_length().is_some_and(|len| len > max_size) {
            return Err(InstallError::Invalid("the release asset is unexpectedly large".into()));
        }
        let io_error = |e| InstallError::Io(path.display().to_string(), e);
        let mut file = tokio::fs::File::create(path).await.map_err(io_error)?;
        let mut hasher = Sha256::new();
        let mut size = 0u64;
        let mut body = response.bytes_stream();
        while let Some(chunk) = body.next().await {
            let chunk = chunk?;
            size += chunk.len() as u64;
            if size > max_size {
                return Err(InstallError::Invalid("the release asset is unexpectedly large".into()));
            }
            hasher.update(&chunk);
            file.write_all(&chunk).await.map_err(io_error)?;
        }
        file.sync_all().await.map_err(io_error)?;
        drop(file);
        let actual = hex::encode(hasher.finalize());
        if actual != expected {
            return Err(InstallError::Invalid(format!("checksum mismatch (expected {expected}, got {actual})")));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).await.map_err(io_error)?;
        }
        Ok(())
    }
}

/// The flag of an installation (one per tool), cleared when it ends however it ends: also when
/// its future is dropped midway (client gone, proxy timeout).
struct Installing<'a> {
    tools: &'a MediaTools,
    flag: &'a AtomicBool,
}

impl<'a> Installing<'a> {
    fn start(tools: &'a MediaTools, flag: &'a AtomicBool) -> Result<Self, InstallError> {
        if flag.swap(true, Ordering::AcqRel) {
            return Err(InstallError::Busy);
        }
        Ok(Self { tools, flag })
    }
}

impl Drop for Installing<'_> {
    fn drop(&mut self) {
        self.tools.invalidate();
        self.flag.store(false, Ordering::Release);
    }
}

/// A binary being installed, removed unless it was moved into place (an installation that failed
/// or was dropped midway).
struct PartialFile(PathBuf);

impl Drop for PartialFile {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("cannot remove {}: {error}", self.0.display());
        }
    }
}

/// A working folder of an installation, removed when it ends.
struct ScratchDir(PathBuf);

impl Drop for ScratchDir {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_dir_all(&self.0)
            && error.kind() != std::io::ErrorKind::NotFound
        {
            tracing::warn!("cannot remove {}: {error}", self.0.display());
        }
    }
}

async fn create_dir(dir: &Path) -> Result<(), InstallError> {
    tokio::fs::create_dir_all(dir).await.map_err(|e| InstallError::Io(dir.display().to_string(), e))
}

/// Copies the binaries of an FFmpeg build out of the `bin` folder of its archive into `dir`, under
/// temporary names. Returns them, `ffmpeg.exe` first, with the name each must take.
fn unpack_ffmpeg(archive: &Path, dir: &Path) -> Result<Vec<(PartialFile, PathBuf)>, InstallError> {
    let io_error = |path: &Path| {
        let path = path.display().to_string();
        move |e| InstallError::Io(path, e)
    };
    let unreadable = |e: zip::result::ZipError| InstallError::Invalid(format!("the FFmpeg archive is unreadable: {e}"));
    let file = File::open(archive).map_err(io_error(archive))?;
    let mut zip = zip::ZipArchive::new(BufReader::new(file)).map_err(unreadable)?;
    let token = &crate::util::random_token()[..8];
    let mut unpacked = Vec::new();
    for binary in FFMPEG_BINARIES {
        let suffix = format!("/bin/{binary}");
        let index = (0..zip.len())
            .find(|&index| zip.name_for_index(index).is_some_and(|name| name.is_ok_and(|name| name.ends_with(&suffix))))
            .ok_or_else(|| InstallError::Invalid(format!("the FFmpeg archive has no {binary}")))?;
        let partial = PartialFile(dir.join(format!("{PARTIAL_PREFIX}{token}-{binary}")));
        let mut entry = zip.by_index(index).map_err(unreadable)?;
        let mut output = File::create(&partial.0).map_err(io_error(&partial.0))?;
        std::io::copy(&mut entry, &mut output).map_err(io_error(&partial.0))?;
        output.sync_all().map_err(io_error(&partial.0))?;
        unpacked.push((partial, dir.join(binary)));
    }
    Ok(unpacked)
}

/// Empties the temporary folder of the tools and removes binaries whose installation was
/// interrupted. Failures are only logged.
fn clean_leftovers(tmp_dir: &Path, bin_dir: &Path) {
    let entries = |dir: &Path| std::fs::read_dir(dir).into_iter().flatten().flatten();
    // The folder itself may be a mount point: only its content is removed.
    let stale = entries(tmp_dir)
        .chain(entries(bin_dir).filter(|entry| entry.file_name().to_string_lossy().starts_with(PARTIAL_PREFIX)));
    for entry in stale {
        let path = entry.path();
        let removed = match entry.file_type() {
            Ok(kind) if kind.is_dir() => std::fs::remove_dir_all(&path),
            _ => std::fs::remove_file(&path),
        };
        if let Err(error) = removed {
            tracing::warn!("cannot remove {}: {error}", path.display());
        }
    }
    if let Err(error) = std::fs::create_dir_all(tmp_dir) {
        tracing::warn!("cannot create {}: {error}", tmp_dir.display());
    }
}

/// Moves the new binary into place. A running Windows executable (or a loaded library) cannot be
/// overwritten but can be renamed, so the old one is moved aside first.
async fn replace(new: &Path, target: &Path) -> std::io::Result<()> {
    let old = target.with_file_name(format!("{}.old", target.file_name().unwrap_or_default().to_string_lossy()));
    if cfg!(windows) && target.exists() {
        let _ = tokio::fs::remove_file(&old).await;
        tokio::fs::rename(target, &old).await?;
    }
    tokio::fs::rename(new, target).await?;
    // Still running on Windows: removed by the next update.
    let _ = tokio::fs::remove_file(&old).await;
    Ok(())
}

/// First line printed by `program <flag>`.
async fn tool_version(program: &Path, flag: &str, tmp_dir: &Path) -> Result<String, String> {
    let args = [OsString::from(flag)];
    let finished = process::run(program, &args, tmp_dir, VERSION_TIMEOUT, &CancellationToken::new())
        .await
        .map_err(|e| e.to_string())?;
    if !finished.status.success() {
        return Err(finished.error().unwrap_or_else(|| format!("exit status {}", finished.status)));
    }
    String::from_utf8_lossy(&finished.stdout)
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "no version printed".into())
}

/// `ffmpeg version 7.1.1-static https://...` → `7.1.1-static`.
fn ffmpeg_version(banner: &str) -> Option<String> {
    banner.strip_prefix("ffmpeg version ")?.split_whitespace().next().map(str::to_owned)
}

pub fn executable(name: &str) -> String {
    if cfg!(windows) { format!("{name}.exe") } else { name.to_owned() }
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    let file = executable(name);
    crate::util::tool_dirs().into_iter().map(|dir| dir.join(&file)).find(|p| p.is_file())
}

/// musl-based Linux (Alpine, the Docker image) needs the `musllinux` builds.
fn is_musl() -> bool {
    cfg!(target_os = "linux")
        && std::fs::read_dir("/lib")
            .map(|entries| entries.flatten().any(|e| e.file_name().to_string_lossy().starts_with("ld-musl-")))
            .unwrap_or(false)
}

/// Name of the official standalone build for a platform (see the yt-dlp release assets).
pub fn asset_name(os: &str, arch: &str, musl: bool) -> Option<&'static str> {
    match (os, arch) {
        ("windows", "x86_64") => Some("yt-dlp.exe"),
        ("windows", "aarch64") => Some("yt-dlp_arm64.exe"),
        ("windows", "x86") => Some("yt-dlp_x86.exe"),
        ("macos", "x86_64" | "aarch64") => Some("yt-dlp_macos"),
        ("linux", "x86_64") => Some(if musl { "yt-dlp_musllinux" } else { "yt-dlp_linux" }),
        ("linux", "aarch64") => Some(if musl { "yt-dlp_musllinux_aarch64" } else { "yt-dlp_linux_aarch64" }),
        _ => None,
    }
}

/// yt-dlp's FFmpeg build for a platform: only Windows, as package managers provide ffmpeg elsewhere.
fn ffmpeg_asset_name(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("windows", "x86_64") => Some("ffmpeg-master-latest-win64-gpl.zip"),
        ("windows", "aarch64") => Some("ffmpeg-master-latest-winarm64-gpl.zip"),
        _ => None,
    }
}

/// Whether [`MediaTools::install_ffmpeg`] works on this platform.
pub fn can_install_ffmpeg() -> bool {
    ffmpeg_asset_name(OS, ARCH).is_some()
}

/// 7-Zip's installer for a platform and its SHA-256: only Windows, as package managers provide
/// 7-Zip elsewhere.
fn seven_zip_installer(os: &str, arch: &str) -> Option<(&'static str, &'static str)> {
    match (os, arch) {
        ("windows", "x86_64") => {
            Some(("7z2604-x64.exe", "d54bf805f9f3704d1e8db2fa3498ae7ef2df0312b40b558e7c71c734430a665d"))
        }
        ("windows", "aarch64") => {
            Some(("7z2604-arm64.exe", "d4117b95495b0e925334ae7b5bddf2853513274603de15dee6acf03999fbc09c"))
        }
        _ => None,
    }
}

/// Whether [`MediaTools::install_seven_zip`] works on this platform.
pub fn can_install_seven_zip() -> bool {
    seven_zip_installer(OS, ARCH).is_some()
}

/// The SHA-256 of `asset` in a `sha256sum`-style listing.
fn checksum_for(sums: &str, asset: &str) -> Option<String> {
    sums.lines().find_map(|line| {
        let (hash, name) = line.trim().split_once(char::is_whitespace)?;
        let name = name.trim_start().trim_start_matches('*');
        (name == asset && hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
            .then(|| hash.to_ascii_lowercase())
    })
}

/// `2026.08.19` → `[2026, 8, 19]` (nightly builds add a fourth number).
fn version_key(version: &str) -> Option<Vec<u64>> {
    version.trim().split('.').map(|part| part.parse().ok()).collect()
}

/// Whether release `tag` is newer than the installed `current` version.
pub fn is_newer(tag: &str, current: Option<&str>) -> bool {
    match (version_key(tag), current.and_then(version_key)) {
        (Some(latest), Some(current)) => latest > current,
        (Some(_), None) => true,
        (None, _) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_release_assets() {
        assert_eq!(asset_name("linux", "x86_64", true), Some("yt-dlp_musllinux"));
        assert_eq!(asset_name("linux", "aarch64", true), Some("yt-dlp_musllinux_aarch64"));
        assert_eq!(asset_name("linux", "x86_64", false), Some("yt-dlp_linux"));
        assert_eq!(asset_name("linux", "aarch64", false), Some("yt-dlp_linux_aarch64"));
        assert_eq!(asset_name("windows", "x86_64", false), Some("yt-dlp.exe"));
        assert_eq!(asset_name("macos", "aarch64", false), Some("yt-dlp_macos"));
        assert_eq!(asset_name("linux", "arm", false), None);
        assert_eq!(asset_name("freebsd", "x86_64", false), None);
    }

    #[test]
    fn reads_checksums() {
        let sums = "1fa6733c37ea6fb51c99ad8fe785e7b7e5f3246c9b980230329d4fb72ed8d4d6  yt-dlp\n\
                    66674953FE251B89F4D08C5F0E35E0728679BD67AB3D7D05C0562AF101DD3E7A *yt-dlp.exe\n\
                    f3dec9cfeaf304cec98290fe41c6ad465d4b747d302473559643e7af24929722  yt-dlp_musllinux\n";
        assert_eq!(
            checksum_for(sums, "yt-dlp.exe").as_deref(),
            Some("66674953fe251b89f4d08c5f0e35e0728679bd67ab3d7d05c0562af101dd3e7a")
        );
        assert_eq!(
            checksum_for(sums, "yt-dlp_musllinux").as_deref(),
            Some("f3dec9cfeaf304cec98290fe41c6ad465d4b747d302473559643e7af24929722")
        );
        assert_eq!(checksum_for(sums, "yt-dlp_linux"), None);
        assert_eq!(checksum_for("short  yt-dlp", "yt-dlp"), None);
    }

    #[test]
    fn compares_versions() {
        assert!(is_newer("2026.08.19", Some("2026.07.30")));
        assert!(is_newer("2026.10.01", Some("2026.9.30")));
        assert!(!is_newer("2026.08.19", Some("2026.08.19")));
        assert!(!is_newer("2026.08.19", Some("2026.08.19.233512")));
        assert!(is_newer("2026.08.19", None));
        assert!(!is_newer("nightly", Some("2026.08.19")));
        let tools = |version: &str| Tools {
            ytdlp: Some(Tool { path: PathBuf::from("yt-dlp"), version: Some(version.into()), managed: false }),
            ffmpeg: None,
        };
        assert!(tools("2025.11.12").supports_js_runtimes());
        assert!(tools("2026.08.19").supports_js_runtimes());
        assert!(!tools("2025.10.22").supports_js_runtimes());
        assert!(!Tools::default().supports_js_runtimes());
    }

    #[test]
    fn cleans_leftovers() {
        let config = tempfile::tempdir().unwrap();
        let (tmp, bin) = (config.path().join("tmp"), config.path().join("bin"));
        std::fs::create_dir_all(tmp.join("_MEI12345").join("lib")).unwrap();
        std::fs::write(tmp.join("_MEI12345").join("lib").join("x.so"), b"x").unwrap();
        std::fs::write(tmp.join("cookies-abc.txt"), b"x").unwrap();
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::write(bin.join(".new-1a2b3c4d-yt-dlp"), b"x").unwrap();
        std::fs::write(bin.join("yt-dlp"), b"x").unwrap();
        std::fs::write(bin.join("ffmpeg"), b"x").unwrap();

        let tools = MediaTools::new(config.path().to_path_buf(), "test").unwrap();
        assert_eq!(tools.tmp_dir(), tmp);
        assert!(tmp.is_dir());
        assert_eq!(std::fs::read_dir(&tmp).unwrap().count(), 0);
        let mut left: Vec<String> =
            std::fs::read_dir(&bin).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
        left.sort();
        assert_eq!(left, ["ffmpeg", "yt-dlp"]);
    }

    #[tokio::test]
    async fn clears_the_installation_state_when_dropped() {
        let config = tempfile::tempdir().unwrap();
        let tools = MediaTools::new(config.path().to_path_buf(), "test").unwrap();
        let partial = config.path().join(".new-test");
        // An installation whose request is abandoned during the download.
        let install = async {
            let _installing = Installing::start(&tools, &tools.updating)?;
            let _partial = PartialFile(partial.clone());
            std::fs::write(&partial, b"x").unwrap();
            assert!(tools.is_updating());
            assert!(matches!(Installing::start(&tools, &tools.updating), Err(InstallError::Busy)));
            // Other tools install independently.
            assert!(Installing::start(&tools, &tools.installing_ffmpeg).is_ok());
            std::future::pending::<()>().await;
            Ok::<_, InstallError>(())
        };
        assert!(tokio::time::timeout(Duration::from_millis(50), install).await.is_err());
        assert!(!tools.is_updating());
        assert!(!partial.exists());
        assert!(Installing::start(&tools, &tools.updating).is_ok());
    }

    #[test]
    fn picks_windows_only_ffmpeg_and_seven_zip_builds() {
        assert_eq!(ffmpeg_asset_name("windows", "x86_64"), Some("ffmpeg-master-latest-win64-gpl.zip"));
        assert_eq!(ffmpeg_asset_name("windows", "aarch64"), Some("ffmpeg-master-latest-winarm64-gpl.zip"));
        assert_eq!(ffmpeg_asset_name("linux", "x86_64"), None);
        assert_eq!(ffmpeg_asset_name("macos", "aarch64"), None);
        assert_eq!(seven_zip_installer("windows", "x86_64").map(|i| i.0), Some("7z2604-x64.exe"));
        assert_eq!(seven_zip_installer("windows", "aarch64").map(|i| i.0), Some("7z2604-arm64.exe"));
        assert_eq!(seven_zip_installer("linux", "x86_64"), None);
        for sum in [SEVEN_ZIP_REDUCED.1, seven_zip_installer("windows", "x86_64").unwrap().1] {
            assert!(sum.len() == 64 && sum.bytes().all(|b| b.is_ascii_hexdigit()));
        }
        // yt-dlp's FFmpeg checksums list the archives by name.
        let sums = "a20f67d27f375cde0a5dc763a83d5b80448a934ef814556ea3f6e353b70c4036  ffmpeg-master-latest-win64-gpl.zip\n\
                    39f861edeed015975257849570d1846281e0e0e4d8ccb0e94726b754cad26510  ffmpeg-master-latest-win64-gpl-shared.zip\n";
        assert_eq!(
            checksum_for(sums, "ffmpeg-master-latest-win64-gpl.zip").as_deref(),
            Some("a20f67d27f375cde0a5dc763a83d5b80448a934ef814556ea3f6e353b70c4036")
        );
    }

    #[test]
    fn unpacks_ffmpeg_from_its_archive() {
        use std::io::Write;

        let dir = tempfile::tempdir().unwrap();
        let archive = dir.path().join("ffmpeg.zip");
        {
            let mut writer = zip::ZipWriter::new(File::create(&archive).unwrap());
            let options = zip::write::SimpleFileOptions::default();
            for (name, content) in [
                ("ffmpeg-master-latest-win64-gpl/LICENSE.txt", "GPL"),
                ("ffmpeg-master-latest-win64-gpl/bin/ffplay.exe", "player"),
                ("ffmpeg-master-latest-win64-gpl/bin/ffprobe.exe", "probe"),
                ("ffmpeg-master-latest-win64-gpl/bin/ffmpeg.exe", "converter"),
            ] {
                writer.start_file(name, options).unwrap();
                writer.write_all(content.as_bytes()).unwrap();
            }
            writer.finish().unwrap();
        }
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();

        let unpacked = unpack_ffmpeg(&archive, &bin).unwrap();
        let targets: Vec<&Path> = unpacked.iter().map(|(_, target)| target.as_path()).collect();
        assert_eq!(targets, [bin.join("ffmpeg.exe"), bin.join("ffprobe.exe")]);
        assert_eq!(std::fs::read_to_string(&unpacked[0].0.0).unwrap(), "converter");
        assert_eq!(std::fs::read_to_string(&unpacked[1].0.0).unwrap(), "probe");
        // Not moved into place: the temporary copies go away.
        drop(unpacked);
        assert_eq!(std::fs::read_dir(&bin).unwrap().count(), 0);

        let empty = dir.path().join("empty.zip");
        zip::ZipWriter::new(File::create(&empty).unwrap()).finish().unwrap();
        assert!(matches!(unpack_ffmpeg(&empty, &bin), Err(InstallError::Invalid(_))));
    }

    #[test]
    fn parses_ffmpeg_banners() {
        assert_eq!(ffmpeg_version("ffmpeg version 6.1.2 Copyright (c) 2000-2024").as_deref(), Some("6.1.2"));
        assert_eq!(
            ffmpeg_version("ffmpeg version n7.1-static https://johnvansickle.com").as_deref(),
            Some("n7.1-static")
        );
        assert_eq!(ffmpeg_version("something else"), None);
    }
}
