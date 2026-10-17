//! What the app remembers between runs, next to the server's own data.

use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

const FILE: &str = "desktop.json";

/// How the app is used, chosen on first start.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Mode {
    /// Rustload downloads on this computer.
    Local,
    /// The app drives a Rustload server elsewhere, with its API key.
    Server { url: String, api_key: String },
}

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    pub mode: Option<Mode>,
    /// Port of the local interface last time: the browser extension keeps working with it.
    pub port: Option<u16>,
    /// Closing the window once explained that Rustload keeps downloading.
    pub background_hint_shown: bool,
    /// The interface window as it was last closed, in logical pixels: the same size at any
    /// display scale.
    pub window: Option<WindowSize>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct WindowSize {
    pub width: f64,
    pub height: f64,
    pub maximized: bool,
}

impl Prefs {
    pub fn load(dir: &Path) -> Self {
        fs::read(dir.join(FILE)).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
    }

    pub fn save(&self, dir: &Path) {
        if let Ok(bytes) = serde_json::to_vec_pretty(self) {
            let _ = fs::write(dir.join(FILE), bytes);
        }
    }
}
