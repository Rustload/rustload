//! Portable mode: a `data` folder next to the program keeps everything the app writes (settings,
//! queue, history, the windows' browser data and sizes), so a copy runs from any folder or USB
//! drive without being installed, and leaves nothing in the user's profile.

use std::{path::PathBuf, sync::OnceLock};

/// The data folder of a portable copy, if this is one.
pub fn data_dir() -> Option<&'static PathBuf> {
    static DIR: OnceLock<Option<PathBuf>> = OnceLock::new();
    DIR.get_or_init(|| {
        // An AppImage runs from a temporary mount: the file the user has is in `APPIMAGE`.
        let program = std::env::var_os("APPIMAGE").map(PathBuf::from).or_else(|| std::env::current_exe().ok())?;
        Some(program.parent()?.join("data")).filter(|dir| dir.is_dir())
    })
    .as_ref()
}

pub fn is_portable() -> bool {
    data_dir().is_some()
}
