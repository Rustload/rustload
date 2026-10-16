//! Updates of the app, from the GitHub releases: checked a minute after start then every six hours,
//! downloaded in the background (the updater only accepts files signed with the project's key) and
//! installed once nothing is downloading and the window is not in front, or right away from the
//! tray menu.

use std::{
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use tauri::{AppHandle, Manager, Wry, menu::MenuItem};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::{WINDOW, text::text};

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
            if let Ok(Some((update, bytes))) = fetch(&app).await {
                let t = text();
                let _ = item.set_text(t.update_item(&update.version));
                let _ = item.set_enabled(true);
                let _ = app.notification().builder().title(t.update_title(&update.version)).body(t.update_body).show();
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

/// The newer version, downloaded and verified, if there is one.
async fn fetch(app: &AppHandle) -> tauri_plugin_updater::Result<Option<(Update, Vec<u8>)>> {
    #[allow(unused_mut)]
    let mut builder = app.updater_builder();
    // Development builds can be pointed at a local `latest.json` to try an update.
    #[cfg(debug_assertions)]
    if let Some(url) = std::env::var("RUSTLOAD_UPDATE_URL").ok().and_then(|url| url.parse().ok()) {
        builder = builder.endpoints(vec![url])?;
    }
    let Some(update) = builder.build()?.check().await? else { return Ok(None) };
    let bytes = update.download(|_, _| {}, || {}).await?;
    Ok(Some((update, bytes)))
}

/// Stops downloading cleanly, installs the downloaded update and starts the new version (on
/// Windows its installer does).
pub async fn install(app: &AppHandle) {
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
