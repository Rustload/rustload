//! The icon in the notification area: live status, pause, and the way to quit.

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use tauri::{
    AppHandle, Manager, Wry,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

use crate::{
    WINDOW,
    backend::{Backend, OFFLINE},
    chooser, magnets, open_window,
    text::text,
    updater,
};

const TRAY: &str = "main";

pub fn create(app: &AppHandle, backend: &Arc<Backend>) -> tauri::Result<()> {
    let t = text();
    // The version, and the install action once an update is downloaded.
    let version = format!("Rustload {}", app.package_info().version);
    let update = MenuItem::with_id(app, "update", version, false, None::<&str>)?;
    let open = MenuItem::with_id(app, "open", t.open, true, None::<&str>)?;
    let pause = MenuItem::with_id(app, "pause", t.pause, true, None::<&str>)?;
    let folder = MenuItem::with_id(app, "folder", t.folder, true, None::<&str>)?;
    let autostart_on = app.autolaunch().is_enabled().unwrap_or(false);
    let autostart = CheckMenuItem::with_id(app, "autostart", t.autostart, true, autostart_on, None::<&str>)?;
    let magnet = CheckMenuItem::with_id(app, "magnet", t.magnet, true, magnets::is_handled(app), None::<&str>)?;
    let mode = MenuItem::with_id(app, "mode", t.change_mode, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", t.quit, true, None::<&str>)?;
    let separator = || PredefinedMenuItem::separator(app);
    let menu = Menu::with_items(app, &[&update, &separator()?, &open, &pause, &separator()?])?;
    // A remote server's downloads folder is on the server.
    if backend.download_root().is_some() {
        menu.append(&folder)?;
    }
    menu.append(&autostart)?;
    // macOS reads the handlers it supports from the app bundle, not at run time.
    if magnets::CAN_REGISTER {
        menu.append(&magnet)?;
    }
    menu.append_items(&[&mode, &separator()?, &quit])?;

    let paused = Arc::new(AtomicBool::new(false));
    let busy = Arc::new(AtomicBool::new(false));
    let on_menu = {
        let (backend, paused, autostart, magnet) = (backend.clone(), paused.clone(), autostart.clone(), magnet.clone());
        move |app: &AppHandle, event: MenuEvent| match event.id().as_ref() {
            "open" => open_window(app, None),
            "pause" => {
                let (backend, pause) = (backend.clone(), !paused.load(Ordering::Relaxed));
                tauri::async_runtime::spawn(async move { backend.set_paused(pause).await });
            }
            "folder" => {
                if let Some(folder) = backend.download_root() {
                    let _ = app.opener().open_path(folder.to_string_lossy(), None::<&str>);
                }
            }
            "autostart" => {
                let launcher = app.autolaunch();
                let _ = if autostart.is_checked().unwrap_or(false) { launcher.enable() } else { launcher.disable() };
                let _ = autostart.set_checked(launcher.is_enabled().unwrap_or(false));
            }
            "magnet" => {
                magnets::set_handled(app, magnet.is_checked().unwrap_or(false));
                let _ = magnet.set_checked(magnets::is_handled(app));
            }
            "mode" => chooser::open(app),
            "update" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move { updater::install(&app).await });
            }
            "quit" => app.exit(0),
            _ => {}
        }
    };
    let icon = app.default_window_icon().cloned().expect("the bundle has an icon");
    TrayIconBuilder::with_id(TRAY)
        .icon(icon)
        .tooltip("Rustload")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(on_menu)
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                open_window(tray.app_handle(), None);
            }
        })
        .build(app)?;

    updater::spawn(app, update, busy.clone());
    tauri::async_runtime::spawn(follow(app.clone(), backend.clone(), pause, paused, busy));
    Ok(())
}

/// Keeps the tooltip and the pause item current and turns finished or failed packages into system
/// notifications, unless the window is in front (the interface shows them then).
async fn follow(
    app: AppHandle,
    backend: Arc<Backend>,
    pause: MenuItem<Wry>,
    paused: Arc<AtomicBool>,
    busy: Arc<AtomicBool>,
) {
    let server = backend.label();
    // Only a server running in the app downloads on this computer.
    let local = backend.download_root().is_some();
    let tooltip = |text: String| {
        if let Some(tray) = app.tray_by_id(TRAY) {
            let _ = tray.set_tooltip(Some(text));
        }
    };
    let mut updates = backend.updates();
    while let Some(update) = updates.recv().await {
        match update.name.as_str() {
            "stats" => {
                tooltip(text().status(Some(&update.data), server.as_deref()));
                busy.store(local && update.data["active"].as_u64().unwrap_or(0) > 0, Ordering::Relaxed);
                let now_paused = update.data["paused"].as_bool().unwrap_or(false);
                if paused.swap(now_paused, Ordering::Relaxed) != now_paused {
                    let _ = pause.set_text(if now_paused { text().resume } else { text().pause });
                }
            }
            OFFLINE => tooltip(text().status(None, server.as_deref())),
            "notice" => {
                let in_front =
                    app.get_webview_window(WINDOW).is_some_and(|window| window.is_focused().unwrap_or(false));
                if let Some(body) = text().notice(&update.data).filter(|_| !in_front) {
                    let _ = app.notification().builder().title("Rustload").body(body).show();
                }
            }
            _ => {}
        }
    }
}
