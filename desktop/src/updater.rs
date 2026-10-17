//! Updates of the app, from the GitHub releases: checked a minute after start then every six hours,
//! downloaded in the background (the updater only accepts files signed with the project's key) and
//! installed once nothing is downloading and the window is not in front, or right away from the
//! tray menu. A portable copy is only told about them: an installer would not replace it.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use tauri::{AppHandle, Manager, Wry, menu::MenuItem};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::{WINDOW, portable, text::text};

/// Where a portable copy's update is downloaded by hand.
const RELEASES: &str = "https://github.com/rustload/rustload/releases/latest";

const FIRST_CHECK: Duration = Duration::from_secs(60);
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);
/// How often a downloaded update looks for a quiet moment to install itself.
const QUIET_POLL: Duration = Duration::from_secs(60);

/// A downloaded update, waiting to be installed.
#[derive(Default)]
struct Ready(Mutex<Option<(Update, Vec<u8>)>>);

/// Starts checking. `item` is the tray menu's version line, which becomes the install action;
/// `busy` tells whether this computer is downloading.
pub fn spawn(app: &AppHandle, item: MenuItem<Wry>, busy: Arc<AtomicBool>) {
    app.manage(Ready::default());
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        tokio::time::sleep(FIRST_CHECK).await;
        loop {
            if portable::is_portable() {
                if let Ok(Some(update)) = check(&app).await {
                    announce(&app, &item, &update.version);
                    return;
                }
            } else if let Ok(Some(update)) = check(&app).await
                && let Ok(bytes) = update.download(|_, _| {}, || {}).await
            {
                announce(&app, &item, &update.version);
                *app.state::<Ready>().0.lock().expect("poisoned") = Some((update, bytes));
                loop {
                    tokio::time::sleep(QUIET_POLL).await;
                    let in_front =
                        app.get_webview_window(WINDOW).is_some_and(|window| window.is_focused().unwrap_or(false));
                    if !busy.load(Ordering::Relaxed) && !in_front {
                        install(&app).await;
                        return;
                    }
                }
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}

/// The newer version, if there is one.
async fn check(app: &AppHandle) -> tauri_plugin_updater::Result<Option<Update>> {
    #[allow(unused_mut)]
    let mut builder = app.updater_builder();
    // Development builds can be pointed at a local `latest.json` to try an update.
    #[cfg(debug_assertions)]
    if let Some(url) = std::env::var("RUSTLOAD_UPDATE_URL").ok().and_then(|url| url.parse().ok()) {
        builder = builder.endpoints(vec![url])?;
    }
    builder.build()?.check().await
}

/// Turns the tray menu's version line into the update action and says so once.
fn announce(app: &AppHandle, item: &MenuItem<Wry>, version: &str) {
    let t = text();
    let portable = portable::is_portable();
    let _ = item.set_text(t.update_item(version, portable));
    let _ = item.set_enabled(true);
    let body = if portable { t.update_body_portable } else { t.update_body };
    let _ = app.notification().builder().title(t.update_title(version)).body(body).show();
}

/// Stops downloading cleanly, installs the downloaded update and starts the new version (on
/// Windows its installer does). A portable copy opens the release page instead.
pub async fn install(app: &AppHandle) {
    if portable::is_portable() {
        let _ = app.opener().open_url(RELEASES, None::<&str>);
        return;
    }
    let Some((update, bytes)) = app.try_state::<Ready>().and_then(|ready| ready.0.lock().expect("poisoned").take())
    else {
        return;
    };
    if let Some(backend) = crate::backend(app) {
        backend.shutdown().await;
    }
    let _ = update.install(bytes);
    // Also after a failed installation: the stopped server starts again.
    app.restart();
}
