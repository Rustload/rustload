//! Diagnostics ("doctor"): quick checks of the installation, with hints to fix what is wrong.
//!
//! Each check is a function returning an [`Outcome`], listed in [`run`] with a stable `id` (the
//! interface translates it) and an English title. Checks run concurrently and each one is bounded
//! by [`CHECK_TIMEOUT`], so the whole report stays under three seconds.

use std::{path::Path, process::Stdio, sync::Arc, time::Duration};

use chrono::{DateTime, Utc};
use futures_util::future::{BoxFuture, FutureExt, join_all};
use serde::Serialize;
use tokio::sync::OnceCell;

use crate::{
    db,
    engine::{
        Engine,
        storage::{category_path, missing_mount},
    },
    models::AccountRow,
    stats::Snapshot,
    util::{human_bytes, random_token},
};

const CHECK_TIMEOUT: Duration = Duration::from_millis(2500);
/// Answers HEAD requests quickly from everywhere; its `Date` header gives the reference time.
const PROBE_URL: &str = "https://www.cloudflare.com/cdn-cgi/trace";
const PROBE_TIMEOUT: Duration = Duration::from_secs(2);
const VERSION_TIMEOUT: Duration = Duration::from_secs(2);
/// Wait for a detection of yt-dlp and ffmpeg, which runs them (see `media::tools`).
const MEDIA_DETECTION_WAIT: Duration = Duration::from_secs(2);
/// Below this, downloads are about to fail.
const LOW_DISK_SPACE: u64 = 1024 * 1024 * 1024;
const MAX_CLOCK_SKEW_SECS: i64 = 60;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Ok,
    Warn,
    Error,
}

/// One line of the report (see `docs/API.md`).
#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub id: &'static str,
    pub status: Status,
    pub title: &'static str,
    pub detail: String,
    pub hint: Option<String>,
}

/// What a check found.
#[derive(Debug, Clone, PartialEq)]
pub struct Outcome {
    status: Status,
    detail: String,
    hint: Option<String>,
}

impl Outcome {
    fn ok(detail: impl Into<String>) -> Self {
        Self { status: Status::Ok, detail: detail.into(), hint: None }
    }

    fn warn(detail: impl Into<String>, hint: Option<&str>) -> Self {
        Self { status: Status::Warn, detail: detail.into(), hint: hint.map(str::to_owned) }
    }

    fn error(detail: impl Into<String>, hint: Option<&str>) -> Self {
        Self { status: Status::Error, detail: detail.into(), hint: hint.map(str::to_owned) }
    }

    fn with_hint(mut self, hint: &str) -> Self {
        self.hint = Some(hint.to_owned());
        self
    }
}

/// What the checks may look at.
pub struct Context<'a> {
    engine: &'a Arc<Engine>,
    /// The request came through a reverse proxy.
    behind_proxy: bool,
    /// The browser reached the proxy over HTTPS.
    https: bool,
    /// One request answers both the `internet` and `clock` checks.
    probe: OnceCell<Result<Probe, String>>,
}

impl<'a> Context<'a> {
    pub fn new(engine: &'a Arc<Engine>, behind_proxy: bool, https: bool) -> Self {
        Self { engine, behind_proxy, https, probe: OnceCell::new() }
    }

    async fn probe(&self) -> &Result<Probe, String> {
        self.probe.get_or_init(|| fetch_probe(&self.engine.api_http)).await
    }
}

/// Runs every check, in display order.
pub async fn run(ctx: &Context<'_>) -> Vec<Check> {
    let config = &ctx.engine.config;
    let checks: Vec<(&'static str, &'static str, BoxFuture<'_, Outcome>)> = vec![
        ("download_dir_writable", "Download folder", writable(&config.download_root).boxed()),
        ("category_folders", "Category folders", category_folders(ctx).boxed()),
        ("config_dir_writable", "Configuration folder", writable(&config.config_dir).boxed()),
        ("disk_space", "Disk space", disk_space(ctx).boxed()),
        ("extractor", "Archive extraction", extractor(ctx).boxed()),
        ("media", "Video site tools", media_tools(ctx).boxed()),
        ("permissions", "File ownership", permissions(ctx).boxed()),
        ("timezone", "Time zone", timezone(ctx).boxed()),
        ("accounts", "Debrid accounts", accounts(ctx).boxed()),
        ("auth", "Authentication", auth(ctx).boxed()),
        ("internet", "Internet access", internet(ctx).boxed()),
        ("clock", "System clock", clock(ctx).boxed()),
    ];
    join_all(checks.into_iter().map(|(id, title, check)| async move {
        let outcome = tokio::time::timeout(CHECK_TIMEOUT, check)
            .await
            .unwrap_or_else(|_| Outcome::warn("The check did not finish in time.", None));
        Check { id, status: outcome.status, title, detail: outcome.detail, hint: outcome.hint }
    }))
    .await
}

const PERMISSION_HINT: &str =
    "The folder must exist and be writable by the user Rustload runs as (PUID/PGID with Docker).";

async fn writable(dir: &Path) -> Outcome {
    let probe = dir.join(format!(".rustload-write-test-{}", &random_token()[..12]));
    match tokio::fs::write(&probe, b"ok").await {
        Ok(()) => {
            let _ = tokio::fs::remove_file(&probe).await;
            Outcome::ok(format!("{} is writable.", dir.display()))
        }
        Err(error) => Outcome::error(format!("Cannot write to {}: {error}.", dir.display()), Some(PERMISSION_HINT)),
    }
}

const MOUNT_HINT: &str = "Mount these folders in the container (volumes of docker-compose.yml) or create them, \
     writable by the user Rustload runs as: the packages sent there fail until then.";

/// Folders outside the download root (Docker volumes, network shares) must exist: Rustload does
/// not create them.
async fn category_folders(ctx: &Context<'_>) -> Outcome {
    let categories = match db::categories::list(&ctx.engine.db).await {
        Ok(categories) => categories,
        Err(error) => return Outcome::warn(format!("Cannot read the categories: {error:#}"), None),
    };
    let root = &ctx.engine.config.download_root;
    let mut problems = Vec::new();
    let mut mounted = 0;
    for category in categories.iter().filter(|c| Path::new(c.path.trim()).is_absolute()) {
        let dir = category_path(root, category);
        if missing_mount(root, category).await.is_some() {
            problems.push(format!("{} ({}) does not exist", category.name, dir.display()));
        } else if writable(&dir).await.status != Status::Ok {
            problems.push(format!("{} ({}) is not writable", category.name, dir.display()));
        } else {
            mounted += 1;
        }
    }
    match (problems.is_empty(), mounted) {
        (false, _) => Outcome::warn(format!("{}.", problems.join("; ")), Some(MOUNT_HINT)),
        (true, 0) => Outcome::ok("Every category is inside the download folder."),
        (true, n) => Outcome::ok(format!("{n} folder(s) outside the download folder, all writable.")),
    }
}

async fn disk_space(ctx: &Context<'_>) -> Outcome {
    let Some(disk) = ctx.engine.disk_info() else {
        return Outcome::warn(
            format!("Cannot read the free space of {}.", ctx.engine.config.download_root.display()),
            None,
        );
    };
    let needed = Snapshot::take(ctx.engine).await.map(|s| s.remaining).unwrap_or(0);
    disk_outcome(disk.free, disk.total, needed)
}

fn disk_outcome(free: u64, total: u64, needed: u64) -> Outcome {
    let percent = if total > 0 { free as f64 * 100.0 / total as f64 } else { 0.0 };
    let detail = format!("{} free of {} ({percent:.0}%).", human_bytes(free), human_bytes(total));
    if free < LOW_DISK_SPACE {
        Outcome::error(detail, Some("Free some space: downloads fail when the disk is full."))
    } else if needed > free {
        Outcome::warn(
            format!("{detail} The queue still needs {}.", human_bytes(needed)),
            Some("Free some space or remove packages from the queue."),
        )
    } else if percent < 5.0 {
        Outcome::warn(detail, Some("The disk is almost full."))
    } else {
        Outcome::ok(detail)
    }
}

async fn extractor(ctx: &Context<'_>) -> Outcome {
    let Some(tool) = ctx.engine.extractor.tool_path() else {
        let hint = if crate::media::tools::can_install_seven_zip() {
            "Install it from Settings › Extraction to extract RAR and 7z archives."
        } else {
            "Install 7-Zip (`7zz` or `7z` in the PATH) to extract RAR and 7z archives. The Docker image includes it."
        };
        return Outcome::warn("7-Zip was not found: only ZIP archives can be extracted.", Some(hint));
    };
    let banner = tokio::process::Command::new(&tool)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .output();
    match tokio::time::timeout(VERSION_TIMEOUT, banner).await {
        Ok(Ok(output)) => {
            let name = describe_tool(&String::from_utf8_lossy(&output.stdout));
            Outcome::ok(format!("{name} ({}).", tool.display()))
        }
        _ => Outcome::warn(
            format!("7-Zip was found at {} but does not run.", tool.display()),
            Some("Check that the 7-Zip binary is executable and matches the system architecture."),
        ),
    }
}

async fn media_tools(ctx: &Context<'_>) -> Outcome {
    let settings = ctx.engine.settings.get();
    if !settings.media.enabled {
        return Outcome::ok("Video sites are turned off in Settings › Video sites.");
    }
    // Detecting the tools runs them, which can take longer than a check may: the last detection is
    // used when there is one, and a new one is left to finish in the background (the next report
    // has it) rather than killed.
    let tools = match ctx.engine.media.last(&settings.media) {
        Some(tools) => tools,
        None => {
            let engine = ctx.engine.clone();
            let detection = tokio::spawn(async move { engine.media.tools(&engine.settings.get().media).await });
            match tokio::time::timeout(MEDIA_DETECTION_WAIT, detection).await {
                Ok(Ok(tools)) => tools,
                _ => {
                    return Outcome::warn(
                        "Still looking for yt-dlp and ffmpeg: run the diagnostics again in a few seconds.",
                        None,
                    );
                }
            }
        }
    };
    let version = |tool: &crate::media::tools::Tool| tool.version.clone().unwrap_or_else(|| "unknown version".into());
    match (tools.ytdlp, tools.ffmpeg) {
        (None, _) => Outcome::warn(
            "yt-dlp was not found: links of video sites cannot be downloaded.",
            Some("Install it from Settings › Video sites, or set its path there. The Docker image includes it."),
        ),
        (Some(ytdlp), None) => Outcome::warn(
            format!("yt-dlp {} found, ffmpeg missing.", version(&ytdlp)),
            Some("Without ffmpeg, videos are limited to formats that need no merging and audio is not converted."),
        ),
        (Some(ytdlp), Some(ffmpeg)) => Outcome::ok(format!("yt-dlp {}, ffmpeg {}.", version(&ytdlp), version(&ffmpeg))),
    }
}

/// `7-Zip 24.09`, or the first line of the banner of a compatible tool (p7zip, NanaZip...).
fn describe_tool(banner: &str) -> String {
    if let Some(version) = parse_7zip_version(banner) {
        return format!("7-Zip {version}");
    }
    banner
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .and_then(|line| line.split(" : ").next())
        .map(|name| name.trim().to_owned())
        .unwrap_or_else(|| "7-Zip".into())
}

/// Version from the banner 7-Zip prints, e.g. `7-Zip (z) 24.09 (x64) : Copyright ...`.
fn parse_7zip_version(banner: &str) -> Option<String> {
    let line = banner.lines().find(|line| line.trim_start().starts_with("7-Zip"))?;
    line.split_whitespace()
        .find(|word| word.starts_with(|c: char| c.is_ascii_digit()) && word.contains('.'))
        .map(str::to_owned)
}

async fn permissions(ctx: &Context<'_>) -> Outcome {
    permissions_outcome(ctx.engine.config.puid)
}

#[cfg(unix)]
fn permissions_outcome(puid: Option<u32>) -> Outcome {
    let (uid, gid) = (nix::unistd::getuid().as_raw(), nix::unistd::getgid().as_raw());
    let detail = format!("Running as uid {uid}, gid {gid}, umask {:03o}.", crate::privileges::umask());
    match puid {
        _ if uid == 0 => Outcome::warn(
            detail,
            Some(
                "Files are created as root. Set PUID and PGID (the output of `id` on the host) so that they belong to your user.",
            ),
        ),
        Some(puid) if puid != uid => {
            Outcome::ok(format!("{detail} PUID/PGID are ignored: the container was started as this user."))
        }
        Some(_) => Outcome::ok(format!("{detail} PUID/PGID are applied.")),
        None => Outcome::ok(detail),
    }
}

#[cfg(not(unix))]
fn permissions_outcome(_puid: Option<u32>) -> Outcome {
    Outcome::ok("File ownership is managed by the operating system (PUID/PGID/UMASK only apply on Linux).")
}

async fn timezone(ctx: &Context<'_>) -> Outcome {
    let tz = std::env::var("TZ").ok().map(|tz| tz.trim().to_owned()).filter(|tz| !tz.is_empty());
    let now = chrono::Local::now();
    timezone_outcome(
        tz.as_deref(),
        &now.format("%Y-%m-%d %H:%M (UTC%:z)").to_string(),
        now.offset().local_minus_utc(),
        ctx.engine.settings.get().schedule.enabled,
        tz.as_deref().and_then(known_zone),
    )
}

/// Whether the tz database has the zone `tz` names; `None` when it cannot be told (POSIX rule
/// strings, systems without `/usr/share/zoneinfo`).
fn known_zone(tz: &str) -> Option<bool> {
    let name = tz.trim_start_matches(':');
    let zoneinfo = Path::new("/usr/share/zoneinfo");
    let is_name = name.contains('/') || !name.chars().any(|c| c.is_ascii_digit() || c == ',');
    (cfg!(unix) && is_name && zoneinfo.is_dir()).then(|| zoneinfo.join(name).is_file())
}

const TZ_HINT: &str = "The download schedule follows this time. Set the TZ environment variable (e.g. Europe/Paris) if it should follow your local time.";

fn timezone_outcome(
    tz: Option<&str>,
    local_time: &str,
    offset_secs: i32,
    schedule_enabled: bool,
    known_zone: Option<bool>,
) -> Outcome {
    match tz {
        Some(tz) if known_zone == Some(false) => Outcome::warn(
            format!("Unknown time zone TZ={tz}: UTC is used. Local time: {local_time}."),
            Some("Use a name of the tz database, such as Europe/Paris or America/New_York."),
        ),
        Some(tz) => Outcome::ok(format!("TZ={tz}. Local time: {local_time}.")),
        None if offset_secs == 0 && schedule_enabled => {
            Outcome::warn(format!("TZ is not set. Local time: {local_time}."), Some(TZ_HINT))
        }
        None if offset_secs == 0 => Outcome::ok(format!("TZ is not set. Local time: {local_time}.")).with_hint(TZ_HINT),
        None => Outcome::ok(format!("Local time: {local_time}.")),
    }
}

async fn accounts(ctx: &Context<'_>) -> Outcome {
    match db::accounts::list(&ctx.engine.db).await {
        Ok(accounts) => accounts_outcome(&accounts),
        Err(error) => Outcome::warn(format!("Cannot read the accounts: {error:#}"), None),
    }
}

fn accounts_outcome(accounts: &[AccountRow]) -> Outcome {
    let enabled: Vec<&AccountRow> = accounts.iter().filter(|a| a.enabled).collect();
    if enabled.is_empty() {
        return Outcome::ok("No debrid account enabled: links are downloaded directly or with the built-in plugins.");
    }
    let failing: Vec<String> =
        enabled.iter().filter_map(|a| a.last_error.as_ref().map(|error| format!("{}: {error}", a.label))).collect();
    if failing.is_empty() {
        Outcome::ok(format!("{} account(s) enabled, no error at the last check.", enabled.len()))
    } else {
        Outcome::warn(
            failing.join("; "),
            Some(
                "Check the account in Settings › Accounts (API key, subscription). Some providers refuse VPN and data-centre addresses.",
            ),
        )
    }
}

async fn auth(ctx: &Context<'_>) -> Outcome {
    let config = &ctx.engine.config;
    auth_outcome(config.auth_enabled, ctx.behind_proxy, ctx.https, config.secure_cookies)
}

fn auth_outcome(auth_enabled: bool, behind_proxy: bool, https: bool, secure_cookies: bool) -> Outcome {
    match (auth_enabled, behind_proxy) {
        (false, true) => Outcome::ok(
            "Authentication is disabled (RUSTLOAD_AUTH=false): the reverse proxy in front of Rustload must authenticate users.",
        ),
        (false, false) => Outcome::warn(
            "Authentication is disabled (RUSTLOAD_AUTH=false) and requests do not come through a reverse proxy: anyone who can reach this address controls Rustload.",
            Some(
                "Remove RUSTLOAD_AUTH=false, or only expose Rustload through a reverse proxy that authenticates users.",
            ),
        ),
        (true, _) if https && !secure_cookies => Outcome::warn(
            "Authentication is enabled, but the session cookie is not marked Secure although the site is served over HTTPS.",
            Some("Set RUSTLOAD_SECURE_COOKIES=true."),
        ),
        (true, _) => Outcome::ok("Authentication is enabled."),
    }
}

/// Result of the request to [`PROBE_URL`].
#[derive(Debug, Clone)]
struct Probe {
    /// `Date` header of the answer.
    server_time: Option<DateTime<Utc>>,
    local_time: DateTime<Utc>,
}

async fn fetch_probe(http: &reqwest::Client) -> Result<Probe, String> {
    let response = http.head(PROBE_URL).timeout(PROBE_TIMEOUT).send().await.map_err(|e| e.without_url().to_string())?;
    let server_time = response
        .headers()
        .get(reqwest::header::DATE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| DateTime::parse_from_rfc2822(v).ok())
        .map(|t| t.with_timezone(&Utc));
    Ok(Probe { server_time, local_time: Utc::now() })
}

async fn internet(ctx: &Context<'_>) -> Outcome {
    internet_outcome(ctx.probe().await)
}

fn internet_outcome(probe: &Result<Probe, String>) -> Outcome {
    match probe {
        Ok(_) => Outcome::ok("Rustload can reach the internet."),
        Err(error) => Outcome::warn(
            format!("Cannot reach the internet: {error}."),
            Some(
                "Check the network, DNS and proxy settings of the host or container: downloads and debrid accounts need internet access.",
            ),
        ),
    }
}

async fn clock(ctx: &Context<'_>) -> Outcome {
    clock_outcome(ctx.probe().await)
}

fn clock_outcome(probe: &Result<Probe, String>) -> Outcome {
    let Ok(Probe { server_time: Some(server_time), local_time }) = probe else {
        return Outcome::warn("Not checked: the internet time could not be read.", None);
    };
    let skew = (*local_time - *server_time).num_seconds();
    if skew.abs() <= MAX_CLOCK_SKEW_SECS {
        return Outcome::ok("The system clock is on time.");
    }
    let direction = if skew > 0 { "ahead of" } else { "behind" };
    Outcome::warn(
        format!("The system clock is {} s {direction} the internet time.", skew.abs()),
        Some("Synchronise the clock of the host (NTP): a wrong clock breaks HTTPS connections and debrid logins."),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const GIB: u64 = 1024 * 1024 * 1024;

    #[test]
    fn reads_7zip_versions() {
        let banner =
            "\n7-Zip (z) 24.09 (x64) : Copyright (c) 1999-2024 Igor Pavlov : 2024-11-29\n\nUsage: 7zz <command>";
        assert_eq!(parse_7zip_version(banner).as_deref(), Some("24.09"));
        let p7zip = "7-Zip [64] 16.02 : Copyright (c) 1999-2016 Igor Pavlov : 2016-05-21";
        assert_eq!(parse_7zip_version(p7zip).as_deref(), Some("16.02"));
        assert_eq!(parse_7zip_version("7-Zip 24.08 (x64) : Copyright (c) 1999-2024").as_deref(), Some("24.08"));
        assert_eq!(parse_7zip_version("something else"), None);
        assert_eq!(describe_tool(banner), "7-Zip 24.09");
        assert_eq!(
            describe_tool(
                "NanaZip 7.0 , version 2609.2 (x64) : (c) M2-Team and Contributors.

Usage: NanaZipC"
            ),
            "NanaZip 7.0 , version 2609.2 (x64)"
        );
        assert_eq!(describe_tool(""), "7-Zip");
    }

    #[test]
    fn judges_disk_space() {
        assert_eq!(disk_outcome(500 * 1024 * 1024, 100 * GIB, 0).status, Status::Error);
        let short = disk_outcome(20 * GIB, 100 * GIB, 30 * GIB);
        assert_eq!(short.status, Status::Warn);
        assert_eq!(short.detail, "20.0 GiB free of 100.0 GiB (20%). The queue still needs 30.0 GiB.");
        assert_eq!(disk_outcome(4 * GIB, 100 * GIB, 0).status, Status::Warn);
        assert_eq!(disk_outcome(50 * GIB, 100 * GIB, GIB), Outcome::ok("50.0 GiB free of 100.0 GiB (50%)."));
    }

    #[test]
    fn judges_time_zones() {
        let at = "2026-10-15 14:03 (UTC+02:00)";
        assert_eq!(timezone_outcome(Some("Europe/Paris"), at, 7200, true, Some(true)).status, Status::Ok);
        assert_eq!(timezone_outcome(Some("Europe/Pariss"), at, 0, false, Some(false)).status, Status::Warn);
        assert_eq!(timezone_outcome(None, at, 0, true, None).status, Status::Warn);
        let unset = timezone_outcome(None, at, 0, false, None);
        assert_eq!((unset.status, unset.hint.is_some()), (Status::Ok, true));
        assert_eq!(timezone_outcome(None, at, 3600, true, None), Outcome::ok(format!("Local time: {at}.")));
    }

    #[test]
    fn reports_account_errors() {
        let account = |label: &str, enabled: bool, error: Option<&str>| AccountRow {
            id: 1,
            provider: "alldebrid".into(),
            label: label.into(),
            credentials: "{}".into(),
            enabled,
            position: 0,
            username: None,
            premium: true,
            premium_until: None,
            traffic_left: None,
            last_check: None,
            last_error: error.map(str::to_owned),
        };
        assert_eq!(accounts_outcome(&[]).status, Status::Ok);
        assert_eq!(accounts_outcome(&[account("A", true, None)]).status, Status::Ok);
        // Disabled accounts do not matter.
        assert_eq!(accounts_outcome(&[account("A", true, None), account("B", false, Some("x"))]).status, Status::Ok);
        let failing = accounts_outcome(&[account("A", true, Some("authentication failed: bad key"))]);
        assert_eq!((failing.status, failing.detail.as_str()), (Status::Warn, "A: authentication failed: bad key"));
    }

    #[test]
    fn judges_authentication() {
        assert_eq!(auth_outcome(true, false, false, false).status, Status::Ok);
        assert_eq!(auth_outcome(true, true, true, false).status, Status::Warn);
        assert_eq!(auth_outcome(true, true, true, true).status, Status::Ok);
        assert_eq!(auth_outcome(false, true, false, false).status, Status::Ok);
        assert_eq!(auth_outcome(false, false, false, false).status, Status::Warn);
    }

    #[test]
    fn compares_the_clock() {
        let now = Utc::now();
        let probe =
            |offset: i64| Ok(Probe { server_time: Some(now - chrono::Duration::seconds(offset)), local_time: now });
        assert_eq!(clock_outcome(&probe(5)).status, Status::Ok);
        assert_eq!(clock_outcome(&probe(300)).detail, "The system clock is 300 s ahead of the internet time.");
        assert_eq!(clock_outcome(&probe(-120)).detail, "The system clock is 120 s behind the internet time.");
        assert_eq!(clock_outcome(&Err("offline".into())).status, Status::Warn);
        assert_eq!(internet_outcome(&Err("dns error".into())).status, Status::Warn);
        assert_eq!(internet_outcome(&probe(0)).status, Status::Ok);
    }

    #[tokio::test]
    async fn checks_folder_writability() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(writable(dir.path()).await.status, Status::Ok);
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        assert_eq!(writable(&dir.path().join("missing")).await.status, Status::Error);
    }
}
