//! File ownership and permissions.
//!
//! Docker images conventionally start as root and switch to the `PUID`/`PGID` given by the user, so
//! that every file Rustload writes belongs to that user. `UMASK` controls the permission bits of
//! new files (`022` → 644/755, `002` → 664/775 for shared groups).

use std::{path::Path, sync::OnceLock};

use anyhow::Result;

use crate::config::Config;

#[cfg_attr(not(unix), allow(dead_code))]
static UMASK: OnceLock<u32> = OnceLock::new();

/// Applies `UMASK` and drops root privileges. Returns human readable notes to log once logging is
/// initialised (this runs before the log directory may be created, so it cannot log itself).
pub fn apply(config: &Config) -> Result<Vec<String>> {
    imp::apply(config)
}

/// Process umask, captured at startup.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn umask() -> u32 {
    *UMASK.get().unwrap_or(&0o022)
}

/// Gives a file or directory the permissions a freshly created one would get, regardless of the
/// mode stored in an archive (some archives carry `0600` or `0700` entries).
pub fn normalize_permissions(path: &Path, is_dir: bool) -> std::io::Result<()> {
    imp::normalize_permissions(path, is_dir)
}

#[cfg(unix)]
mod imp {
    use std::{fs, os::unix::fs::PermissionsExt, path::Path};

    #[cfg(not(target_vendor = "apple"))]
    use anyhow::Context;
    use anyhow::Result;
    #[cfg(not(target_vendor = "apple"))]
    use nix::unistd::{chown, setgid, setgroups, setuid};
    use nix::{
        sys::stat::{Mode, umask},
        unistd::{Gid, Uid, getuid},
    };

    use super::UMASK;
    use crate::config::Config;

    pub fn apply(config: &Config) -> Result<Vec<String>> {
        let mut notes = Vec::new();

        let mask = match config.umask {
            Some(mask) => {
                umask(Mode::from_bits_truncate(mask as nix::libc::mode_t));
                notes.push(format!("umask set to {mask:03o}"));
                mask
            }
            None => {
                let current = umask(Mode::empty());
                umask(current);
                current.bits() as u32
            }
        };
        let _ = UMASK.set(mask);

        if !getuid().is_root() {
            return Ok(notes);
        }
        let Some(uid) = config.puid else {
            notes.push("running as root: set PUID/PGID so that downloaded files belong to your user".into());
            return Ok(notes);
        };
        let gid = config.pgid.unwrap_or(uid);
        let (uid, gid) = (Uid::from_raw(uid), Gid::from_raw(gid));
        drop_privileges(config, uid, gid)?;
        notes.push(format!("dropped privileges to uid={uid} gid={gid}"));
        Ok(notes)
    }

    #[cfg(not(target_vendor = "apple"))]
    fn drop_privileges(config: &Config, uid: Uid, gid: Gid) -> Result<()> {
        for dir in [&config.config_dir, &config.download_root] {
            fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        }
        // The state directory is small and fully ours: fix it recursively (files may have been
        // created by an earlier run as root). The download root is only fixed at the top level.
        chown_recursive(&config.config_dir, uid, gid)?;
        chown(&config.download_root, Some(uid), Some(gid))
            .with_context(|| format!("cannot chown {}", config.download_root.display()))?;

        setgroups(&[gid]).context("setgroups failed")?;
        setgid(gid).context("setgid failed")?;
        setuid(uid).context("setuid failed")?;
        Ok(())
    }

    /// macOS offers no safe way to clear root's supplementary groups, and `PUID`/`PGID` are meant
    /// for Linux containers: refuse rather than run with root's groups.
    #[cfg(target_vendor = "apple")]
    fn drop_privileges(_: &Config, _: Uid, _: Gid) -> Result<()> {
        anyhow::bail!("PUID/PGID are supported on Linux only: start Rustload as the user who should own the files")
    }

    #[cfg(not(target_vendor = "apple"))]
    fn chown_recursive(path: &Path, uid: Uid, gid: Gid) -> Result<()> {
        chown(path, Some(uid), Some(gid)).with_context(|| format!("cannot chown {}", path.display()))?;
        if path.is_dir() {
            for entry in fs::read_dir(path)? {
                let entry = entry?;
                if !entry.file_type()?.is_symlink() {
                    chown_recursive(&entry.path(), uid, gid)?;
                }
            }
        }
        Ok(())
    }

    pub fn normalize_permissions(path: &Path, is_dir: bool) -> std::io::Result<()> {
        let base = if is_dir { 0o777 } else { 0o666 };
        fs::set_permissions(path, fs::Permissions::from_mode(base & !super::umask()))
    }
}

#[cfg(not(unix))]
mod imp {
    use std::path::Path;

    use anyhow::Result;

    use crate::config::Config;

    pub fn apply(config: &Config) -> Result<Vec<String>> {
        if config.puid.is_some() || config.pgid.is_some() || config.umask.is_some() {
            return Ok(vec!["PUID/PGID/UMASK are only supported on Linux; ignored".into()]);
        }
        Ok(Vec::new())
    }

    pub fn normalize_permissions(_path: &Path, _is_dir: bool) -> std::io::Result<()> {
        Ok(())
    }
}
