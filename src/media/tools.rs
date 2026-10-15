//! Locating yt-dlp and ffmpeg, and installing or updating the managed yt-dlp binary.
//!
//! Lookup order: the path set in the settings, then the managed copy in `<config>/bin/` (installed
//! from the interface and kept up to date), then `PATH`. Detection runs the tools once (`--version`)
//! and is cached, as starting a PyInstaller bundle takes a second.
//!
//! yt-dlp runs with `<config>/tmp` as its temporary folder (see `process::command`), emptied at
//! startup: runs that were killed leave their unpacked build there.

use std::{
    ffi::OsString,
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
    #[error("no official yt-dlp build exists for this platform ({0}); install yt-dlp yourself and set its path")]
    Unsupported(String),
    #[error("cannot reach GitHub: {0}")]
    Network(String),
    #[error("{0}")]
    Invalid(String),
    #[error("cannot write {0}: {1}")]
    Io(String, std::io::Error),
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
    updating: AtomicBool,
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
        let _installing = Installing::start(self)?;
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
        let _installing = Installing::start(self)?;
        self.install(release).await.map(Some)
    }

    async fn install(&self, release: &Release) -> Result<String, InstallError> {
        let platform = format!("{}/{}", std::env::consts::OS, std::env::consts::ARCH);
        let name = asset_name(std::env::consts::OS, std::env::consts::ARCH, is_musl())
            .ok_or_else(|| InstallError::Unsupported(platform.clone()))?;
        let find = |wanted: &str| release.assets.iter().find(|a| a.name == wanted);
        let asset = find(name).ok_or_else(|| InstallError::Unsupported(platform.clone()))?;
        let sums = find(CHECKSUMS_ASSET).ok_or_else(|| {
            InstallError::Invalid("the release has no checksum file: refusing an unverified binary".into())
        })?;
        let sums = self.http.get(&sums.browser_download_url).send().await?.error_for_status()?.text().await?;
        let expected = checksum_for(&sums, name)
            .ok_or_else(|| InstallError::Invalid(format!("no checksum published for {name}")))?;

        let dir = self.bin_dir();
        tokio::fs::create_dir_all(&dir).await.map_err(|e| InstallError::Io(dir.display().to_string(), e))?;
        let target = self.managed_path();
        let file_name = target.file_name().unwrap_or_default().to_string_lossy();
        let temp = PartialFile(dir.join(format!("{PARTIAL_PREFIX}{}-{file_name}", &crate::util::random_token()[..8])));
        self.download_verified(&asset.browser_download_url, &temp.0, &expected).await?;
        let version = tool_version(&temp.0, "--version", &self.tmp_dir())
            .await
            .map_err(|e| InstallError::Invalid(format!("the downloaded yt-dlp does not run here: {e}")))?;
        replace(&temp.0, &target).await.map_err(|e| InstallError::Io(target.display().to_string(), e))?;
        tracing::info!("yt-dlp {version} installed in {}", target.display());
        Ok(version)
    }

    async fn download_verified(&self, url: &str, path: &Path, expected: &str) -> Result<(), InstallError> {
        let response = self.http.get(url).send().await?.error_for_status()?;
        if response.content_length().is_some_and(|len| len > MAX_BINARY_SIZE) {
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
            if size > MAX_BINARY_SIZE {
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

/// The `updating` flag of an installation, cleared when it ends however it ends: also when its
/// future is dropped midway (client gone, proxy timeout).
struct Installing<'a>(&'a MediaTools);

impl<'a> Installing<'a> {
    fn start(tools: &'a MediaTools) -> Result<Self, InstallError> {
        if tools.updating.swap(true, Ordering::AcqRel) {
            return Err(InstallError::Busy);
        }
        Ok(Self(tools))
    }
}

impl Drop for Installing<'_> {
    fn drop(&mut self) {
        self.0.invalidate();
        self.0.updating.store(false, Ordering::Release);
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

/// Moves the new binary into place. A running Windows executable cannot be overwritten but can
/// be renamed, so the old one is moved aside first.
async fn replace(new: &Path, target: &Path) -> std::io::Result<()> {
    let old = target.with_extension("old");
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
    std::env::var_os("PATH")
        .and_then(|paths| std::env::split_paths(&paths).map(|dir| dir.join(&file)).find(|p| p.is_file()))
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
            let _installing = Installing::start(&tools)?;
            let _partial = PartialFile(partial.clone());
            std::fs::write(&partial, b"x").unwrap();
            assert!(tools.is_updating());
            assert!(matches!(Installing::start(&tools), Err(InstallError::Busy)));
            std::future::pending::<()>().await;
            Ok::<_, InstallError>(())
        };
        assert!(tokio::time::timeout(Duration::from_millis(50), install).await.is_err());
        assert!(!tools.is_updating());
        assert!(!partial.exists());
        assert!(Installing::start(&tools).is_ok());
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
