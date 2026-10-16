//! The app's own words (tray menu, tooltip, system notifications) in the system's language, worded
//! like the interface.

use std::sync::OnceLock;

use serde_json::Value;

pub struct Text {
    pub open: &'static str,
    pub pause: &'static str,
    pub resume: &'static str,
    pub folder: &'static str,
    pub autostart: &'static str,
    pub magnet: &'static str,
    pub change_mode: &'static str,
    pub quit: &'static str,
    pub background_title: &'static str,
    pub background_body: &'static str,
    pub start_failed: &'static str,
    french: bool,
}

const EN: Text = Text {
    open: "Open Rustload",
    pause: "Pause all",
    resume: "Resume all",
    folder: "Open the downloads folder",
    autostart: "Start with the computer",
    magnet: "Open magnet links",
    change_mode: "Change mode…",
    quit: "Quit Rustload",
    background_title: "Rustload keeps downloading",
    background_body: "Its icon in the notification area opens it again or quits it.",
    start_failed: "Rustload could not start.",
    french: false,
};

const FR: Text = Text {
    open: "Ouvrir Rustload",
    pause: "Tout mettre en pause",
    resume: "Tout reprendre",
    folder: "Ouvrir le dossier des téléchargements",
    autostart: "Lancer au démarrage de l’ordinateur",
    magnet: "Ouvrir les liens magnet",
    change_mode: "Changer de mode…",
    quit: "Quitter Rustload",
    background_title: "Rustload continue de télécharger",
    background_body: "Son icône dans la zone de notification le rouvre ou le quitte.",
    start_failed: "Rustload n’a pas pu démarrer.",
    french: true,
};

pub fn text() -> &'static Text {
    static TEXT: OnceLock<&'static Text> = OnceLock::new();
    TEXT.get_or_init(|| match sys_locale::get_locale() {
        Some(locale) if locale.to_ascii_lowercase().starts_with("fr") => &FR,
        _ => &EN,
    })
}

impl Text {
    /// The tooltip of the tray icon, from a `stats` event (`None` while the server is unreachable).
    /// `server` names a remote server.
    pub fn status(&self, stats: Option<&Value>, server: Option<&str>) -> String {
        let name = match server {
            Some(server) => format!("Rustload ({server})"),
            None => "Rustload".to_owned(),
        };
        let Some(stats) = stats else {
            return format!("{name} — {}", if self.french { "serveur injoignable" } else { "server unreachable" });
        };
        let count = |key: &str| stats[key].as_u64().unwrap_or(0);
        if count("active") > 0 {
            let speed = self.speed(count("speed"));
            return if self.french {
                format!("{name} télécharge à {speed}")
            } else {
                format!("{name} is downloading at {speed}")
            };
        }
        let state = match (stats["paused"].as_bool().unwrap_or(false), count("queued") > 0, self.french) {
            (true, _, false) => "Everything is paused",
            (true, _, true) => "Tout est en pause",
            (false, true, false) => "Downloads are about to start",
            (false, true, true) => "Les téléchargements vont démarrer",
            (false, false, false) => "Nothing to download",
            (false, false, true) => "Rien à télécharger",
        };
        format!("{name} — {state}")
    }

    /// A speed as the interface shows it: "24.0 MiB/s", "24,0 Mio/s".
    pub fn speed(&self, bytes_per_second: u64) -> String {
        const UNITS: [[&str; 5]; 2] = [["B", "KiB", "MiB", "GiB", "TiB"], ["o", "Kio", "Mio", "Gio", "Tio"]];
        let mut size = bytes_per_second as f64;
        let mut unit = 0;
        while size >= 1024.0 && unit < 4 {
            size /= 1024.0;
            unit += 1;
        }
        let digits = if unit == 0 {
            0
        } else if size < 10.0 {
            2
        } else if size < 100.0 {
            1
        } else {
            0
        };
        let mut value = format!("{size:.digits$}");
        if self.french {
            value = value.replace('.', ",");
        }
        format!("{value}\u{a0}{}/s", UNITS[usize::from(self.french)][unit])
    }

    /// The text of a `notice` event worth a system notification, or `None`.
    pub fn notice(&self, notice: &Value) -> Option<String> {
        if !matches!(notice["level"].as_str(), Some("success" | "error")) {
            return None;
        }
        let param = |name: &str| notice["params"][name].as_str().unwrap_or_default();
        let (name, error, file) = (param("name"), param("error"), param("file"));
        let text = match (notice["key"].as_str().unwrap_or_default(), self.french) {
            ("package_finished", false) => format!("“{name}” finished downloading"),
            ("package_finished", true) => format!("«\u{a0}{name}\u{a0}» est téléchargé"),
            ("package_failed" | "download_failed", false) => format!("“{name}” failed: {error}"),
            ("package_failed" | "download_failed", true) => format!("«\u{a0}{name}\u{a0}» a échoué\u{a0}: {error}"),
            ("checksum_failed", false) => format!("“{file}” in “{name}” is corrupted: its checksum doesn’t match"),
            ("checksum_failed", true) => format!(
                "«\u{a0}{file}\u{a0}» dans «\u{a0}{name}\u{a0}» est corrompu\u{a0}: sa somme de contrôle ne correspond pas"
            ),
            _ => notice["message"].as_str().unwrap_or_default().to_owned(),
        };
        Some(text)
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn formats_speeds_like_the_interface() {
        assert_eq!(EN.speed(0), "0\u{a0}B/s");
        assert_eq!(EN.speed(25_165_824), "24.0\u{a0}MiB/s");
        assert_eq!(FR.speed(25_165_824), "24,0\u{a0}Mio/s");
        assert_eq!(EN.speed(5 * 1024 * 1024 + 1), "5.00\u{a0}MiB/s");
    }

    #[test]
    fn describes_the_queue() {
        let stats = json!({ "active": 2, "speed": 1024 });
        assert_eq!(EN.status(Some(&stats), None), "Rustload is downloading at 1.00\u{a0}KiB/s");
        assert_eq!(FR.status(Some(&json!({ "active": 0, "paused": true })), None), "Rustload — Tout est en pause");
        assert_eq!(EN.status(Some(&json!({})), Some("nas")), "Rustload (nas) — Nothing to download");
        assert_eq!(FR.status(None, Some("nas")), "Rustload (nas) — serveur injoignable");
    }

    #[test]
    fn notifies_finished_and_failed_packages_only() {
        let finished = json!({ "level": "success", "key": "package_finished", "params": { "name": "Film" } });
        assert_eq!(EN.notice(&finished).as_deref(), Some("“Film” finished downloading"));
        assert!(EN.notice(&json!({ "level": "info", "key": "queue_finished" })).is_none());
    }
}
