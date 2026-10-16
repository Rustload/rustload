//! Opening `magnet:` links with Rustload. On Windows and Linux it is a choice in the tray menu, as
//! it takes them from a torrent client; on macOS the app bundle declares them.

use tauri::AppHandle;
use tauri_plugin_deep_link::DeepLinkExt;

/// Whether the handler can be changed while the app runs.
pub const CAN_REGISTER: bool = cfg!(any(windows, target_os = "linux"));
const SCHEME: &str = "magnet";

pub fn is_handled(app: &AppHandle) -> bool {
    CAN_REGISTER && app.deep_link().is_registered(SCHEME).unwrap_or(false)
}

pub fn set_handled(app: &AppHandle, handled: bool) {
    let links = app.deep_link();
    let _ = if handled { links.register(SCHEME) } else { links.unregister(SCHEME) };
}

pub fn is_magnet(link: &str) -> bool {
    link.get(..7).is_some_and(|scheme| scheme.eq_ignore_ascii_case("magnet:"))
}
