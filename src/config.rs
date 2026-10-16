//! Process-level configuration read from environment variables.
//!
//! Everything a user may want to change at runtime lives in [`crate::settings`]; this module only
//! holds what must be known before the database is opened (paths, ports, bootstrap credentials).

use std::{
    env,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    path::PathBuf,
};

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone)]
pub struct Config {
    /// Address of the web UI / API.
    pub bind: SocketAddr,
    /// Address of the Click'n'Load listener (only started when enabled in settings).
    pub cnl_bind: SocketAddr,
    /// Database, logs and other persistent state.
    pub config_dir: PathBuf,
    /// Root folder for downloads; relative category paths are resolved against it.
    pub download_root: PathBuf,
    /// Disable the login screen (e.g. when an authenticating reverse proxy sits in front).
    pub auth_enabled: bool,
    /// Optional bootstrap admin account, created on first start when no user exists.
    pub admin_user: Option<String>,
    pub admin_password: Option<String>,
    /// Mark the session cookie as `Secure` (set when served over HTTPS).
    pub secure_cookies: bool,
    /// `tracing` filter directive.
    pub log_filter: String,
    /// Linux user/group to run as when started as root (Docker `PUID`/`PGID`).
    pub puid: Option<u32>,
    pub pgid: Option<u32>,
    /// File creation mask, octal (Docker `UMASK`, e.g. `022` or `002`).
    pub umask: Option<u32>,
    /// Embedded in the desktop app (see [`Config::desktop`]).
    pub desktop: bool,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        let host: IpAddr = var("RUSTLOAD_HOST")
            .map(|h| h.parse().with_context(|| format!("invalid RUSTLOAD_HOST `{h}`")))
            .transpose()?
            .unwrap_or(IpAddr::V4(Ipv4Addr::UNSPECIFIED));
        let port = parse_var("RUSTLOAD_PORT")?.unwrap_or(8800);
        // Click'n'Load accepts unauthenticated requests: loopback only unless asked otherwise.
        let cnl_host: IpAddr = var("RUSTLOAD_CNL_HOST")
            .map(|h| h.parse().with_context(|| format!("invalid RUSTLOAD_CNL_HOST `{h}`")))
            .transpose()?
            .unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
        let cnl_port = parse_var("RUSTLOAD_CNL_PORT")?.unwrap_or(9666);

        let umask = match var("UMASK") {
            Some(raw) => Some(
                u32::from_str_radix(raw.trim_start_matches("0o"), 8)
                    .ok()
                    .filter(|m| *m <= 0o777)
                    .with_context(|| format!("invalid UMASK `{raw}` (expected octal like 022)"))?,
            ),
            None => None,
        };

        let admin_user = var("RUSTLOAD_ADMIN_USER");
        let admin_password = var("RUSTLOAD_ADMIN_PASSWORD");
        if admin_user.is_some() != admin_password.is_some() {
            bail!("RUSTLOAD_ADMIN_USER and RUSTLOAD_ADMIN_PASSWORD must be set together");
        }

        Ok(Self {
            bind: SocketAddr::new(host, port),
            cnl_bind: SocketAddr::new(cnl_host, cnl_port),
            config_dir: absolute(var("RUSTLOAD_CONFIG_DIR").unwrap_or_else(|| "./config".into()))?,
            download_root: absolute(var("RUSTLOAD_DOWNLOAD_DIR").unwrap_or_else(|| "./downloads".into()))?,
            auth_enabled: parse_bool("RUSTLOAD_AUTH")?.unwrap_or(true),
            admin_user,
            admin_password,
            secure_cookies: parse_bool("RUSTLOAD_SECURE_COOKIES")?.unwrap_or(false),
            log_filter: var("RUSTLOAD_LOG").or_else(|| var("RUST_LOG")).unwrap_or_else(|| "info".into()),
            puid: parse_var("PUID")?,
            pgid: parse_var("PGID")?,
            umask,
            desktop: false,
        })
    }

    /// The desktop app's configuration: the interface answers on this computer only, through
    /// `port`, and opens logged in to an account named after `username` whose random password
    /// nobody needs (see `Server::login_url`). Click'n'Load keeps its usual port.
    pub fn desktop(config_dir: PathBuf, download_root: PathBuf, port: u16, username: &str) -> Self {
        let localhost = IpAddr::V4(Ipv4Addr::LOCALHOST);
        Self {
            bind: SocketAddr::new(localhost, port),
            cnl_bind: SocketAddr::new(localhost, 9666),
            config_dir,
            download_root,
            auth_enabled: true,
            admin_user: Some(username.to_owned()),
            admin_password: Some(crate::util::random_token()),
            secure_cookies: false,
            log_filter: var("RUSTLOAD_LOG").unwrap_or_else(|| "info".into()),
            puid: None,
            pgid: None,
            umask: None,
            desktop: true,
        }
    }

    pub fn database_path(&self) -> PathBuf {
        self.config_dir.join("rustload.db")
    }

    pub fn logs_dir(&self) -> PathBuf {
        self.config_dir.join("logs")
    }
}

/// Paths are made absolute once so that logs and the UI show unambiguous locations.
fn absolute(path: String) -> Result<PathBuf> {
    std::path::absolute(&path).with_context(|| format!("invalid path `{path}`"))
}

fn var(name: &str) -> Option<String> {
    env::var(name).ok().map(|v| v.trim().to_owned()).filter(|v| !v.is_empty())
}

fn parse_var<T: std::str::FromStr>(name: &str) -> Result<Option<T>> {
    var(name).map(|raw| raw.parse::<T>().ok().with_context(|| format!("invalid value for {name}: `{raw}`"))).transpose()
}

fn parse_bool(name: &str) -> Result<Option<bool>> {
    var(name)
        .map(|raw| match raw.to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            _ => bail!("invalid boolean for {name}: `{raw}`"),
        })
        .transpose()
}

#[cfg(test)]
impl Config {
    /// A configuration whose folders live in `dir`.
    pub fn for_tests(dir: &std::path::Path) -> Self {
        Self {
            bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            cnl_bind: SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
            config_dir: dir.join("config"),
            download_root: dir.join("downloads"),
            auth_enabled: false,
            admin_user: None,
            admin_password: None,
            secure_cookies: false,
            log_filter: "info".into(),
            puid: None,
            pgid: None,
            umask: None,
            desktop: false,
        }
    }
}
