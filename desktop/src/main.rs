//! Rustload Desktop: Rustload in a window of its own, with an icon in the notification area that
//! keeps going once the window is closed. It downloads on this computer, or drives a Rustload
//! server elsewhere (gateway mode).

// No console next to the app on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod backend;
mod chooser;
mod magnets;
mod prefs;
mod text;
mod tray;
mod updater;

use std::{
    net::{Ipv4Addr, TcpListener},
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock},
};

use anyhow::Context;
use rustload::{Config, Server};
use tauri::{
    AppHandle, Manager, RunEvent, Url, WebviewUrl, WebviewWindowBuilder, WindowEvent, webview::NewWindowResponse,
};
use tauri_plugin_dialog::{DialogExt, MessageDialogKind};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

use crate::{
    backend::Backend,
    prefs::{Mode, Prefs},
    text::text,
};

/// Label of the interface's window.
const WINDOW: &str = "main";
/// The local interface's port unless it is taken: the browser extension's default address.
const DEFAULT_PORT: u16 = 8800;
/// Given by the start-with-the-computer entry: stay in the notification area.
const HIDDEN_FLAG: &str = "--hidden";

struct Desktop {
    /// Set once a mode is chosen and started.
    backend: OnceLock<Arc<Backend>>,
    data_dir: PathBuf,
    prefs: Mutex<Prefs>,
}

fn main() {
    let app = tauri::Builder::default()
        // Opening Rustload again, or a magnet link, goes to the instance already running.
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| open_args(app, &args, true)))
        .plugin(tauri_plugin_deep_link::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![HIDDEN_FLAG]),
        ))
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_window_state::Builder::new().build())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .invoke_handler(tauri::generate_handler![chooser::current, chooser::use_local, chooser::use_server])
        .setup(|app| {
            if let Err(error) = start(app.handle()) {
                let handle = app.handle().clone();
                app.dialog()
                    .message(format!("{}\n\n{error:#}", text().start_failed))
                    .kind(MessageDialogKind::Error)
                    .title("Rustload")
                    .show(move |_| handle.exit(1));
            }
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == WINDOW && matches!(event, WindowEvent::CloseRequested { .. }) {
                explain_background(window.app_handle());
            }
        })
        .build(tauri::generate_context!())
        .expect("cannot build the app");

    app.run(|app, event| match event {
        // Closing the window keeps going: only *Quit* (or the system) ends the app. Before a mode is
        // chosen, closing the chooser ends it.
        RunEvent::ExitRequested { code: None, api, .. } if backend(app).is_some() => api.prevent_exit(),
        RunEvent::Exit => stop(app),
        #[cfg(target_os = "macos")]
        RunEvent::Reopen { .. } => open_window(app, None),
        _ => {}
    });
}

fn start(app: &AppHandle) -> anyhow::Result<()> {
    let data_dir = app.path().app_local_data_dir().context("no folder for the app's data")?;
    std::fs::create_dir_all(&data_dir).with_context(|| format!("cannot create {}", data_dir.display()))?;
    let prefs = Prefs::load(&data_dir);
    let mode = prefs.mode.clone();
    app.manage(Desktop { backend: OnceLock::new(), data_dir: data_dir.clone(), prefs: Mutex::new(prefs) });
    let Some(mode) = mode else {
        chooser::open(app);
        return Ok(());
    };

    let backend = Arc::new(match mode {
        Mode::Local => Backend::local(start_local(app, &data_dir)?),
        Mode::Server { url, api_key } => Backend::Remote(backend::remote(&url, &api_key)?),
    });
    let _ = app.state::<Desktop>().backend.set(backend.clone());
    tray::create(app, &backend)?;

    // macOS hands magnet links over as events; Windows and Linux on the command line.
    #[cfg(target_os = "macos")]
    {
        use tauri_plugin_deep_link::DeepLinkExt;
        let handle = app.clone();
        app.deep_link().on_open_url(move |event| {
            let links: Vec<String> = event.urls().iter().map(Url::to_string).collect();
            open_args(&handle, &links, true);
        });
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    open_args(app, &args, !args.iter().any(|arg| arg == HIDDEN_FLAG));
    Ok(())
}

/// Starts Rustload on this computer, downloading into the user's downloads folder.
fn start_local(app: &AppHandle, data_dir: &Path) -> anyhow::Result<Server> {
    let downloads = app.path().download_dir().context("no downloads folder")?.join("Rustload");
    let desktop = app.state::<Desktop>();
    let saved_port = desktop.prefs.lock().expect("poisoned").port;
    let config =
        Config::desktop(data_dir.to_path_buf(), downloads, free_port(saved_port.unwrap_or(DEFAULT_PORT)), &username());
    let server = tauri::async_runtime::block_on(Server::start(config))?;
    let port = server.handle().address().port();
    if saved_port != Some(port) {
        let mut prefs = desktop.prefs.lock().expect("poisoned");
        prefs.port = Some(port);
        prefs.save(data_dir);
    }
    Ok(server)
}

fn backend(app: &AppHandle) -> Option<Arc<Backend>> {
    app.try_state::<Desktop>()?.backend.get().cloned()
}

/// Adds the magnet links among `args` through the add dialog, or shows the window if `show`.
fn open_args(app: &AppHandle, args: &[String], show: bool) {
    let magnets: Vec<&str> = args.iter().map(String::as_str).filter(|arg| magnets::is_magnet(arg)).collect();
    if !magnets.is_empty() {
        let links: String = url::form_urlencoded::byte_serialize(magnets.join("\n").as_bytes()).collect();
        open_window(app, Some(&format!("/?add={links}")));
    } else if show {
        open_window(app, None);
    }
}

/// Shows the interface, at `path` if given (the local one opens logged in). Before a mode is
/// chosen, shows the chooser instead.
pub fn open_window(app: &AppHandle, path: Option<&str>) {
    let Some(backend) = backend(app) else {
        chooser::open(app);
        return;
    };
    if let Some(window) = app.get_webview_window(WINDOW) {
        if let Some(url) = path.and_then(|path| backend.page(path)) {
            let _ = window.navigate(url);
        }
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let Some(url) = backend.page(path.unwrap_or("/")) else { return };
    let data_dir = app.state::<Desktop>().data_dir.join("webview");
    let (navigating, opening) = ((app.clone(), backend.clone()), app.clone());
    let built = WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::External(url))
        .title("Rustload")
        .inner_size(1280.0, 820.0)
        .min_inner_size(420.0, 560.0)
        .center()
        .data_directory(data_dir)
        // The interface takes dropped .torrent files itself.
        .disable_drag_drop_handler()
        // Links to other sites (documentation, debrid services) open in the browser.
        .on_navigation(move |url| navigating.1.is_interface(url) || open_in_browser(&navigating.0, url))
        .on_new_window(move |url, _| {
            open_in_browser(&opening, &url);
            NewWindowResponse::Deny
        })
        .build();
    if let Ok(window) = built {
        let _ = window.set_focus();
    }
}

/// Always `false`: the window stays on the interface.
fn open_in_browser(app: &AppHandle, url: &Url) -> bool {
    if matches!(url.scheme(), "http" | "https" | "mailto") {
        let _ = app.opener().open_url(url.as_str(), None::<&str>);
    }
    false
}

/// The first time the window closes, says that Rustload keeps going in the background.
fn explain_background(app: &AppHandle) {
    let Some(desktop) = app.try_state::<Desktop>() else { return };
    let mut prefs = desktop.prefs.lock().expect("poisoned");
    if prefs.background_hint_shown {
        return;
    }
    let t = text();
    let _ = app.notification().builder().title(t.background_title).body(t.background_body).show();
    prefs.background_hint_shown = true;
    prefs.save(&desktop.data_dir);
}

/// Stops downloading cleanly before the app ends.
fn stop(app: &AppHandle) {
    if let Some(backend) = backend(app) {
        backend.stop();
    }
}

/// `preferred` if free, else the next free port, else one the system picks. A server listening on
/// all addresses (Rustload in Docker) counts: Windows would let the app take its local connections.
fn free_port(preferred: u16) -> u16 {
    let free = |port: u16| {
        [Ipv4Addr::UNSPECIFIED, Ipv4Addr::LOCALHOST].iter().all(|ip| TcpListener::bind((*ip, port)).is_ok())
    };
    std::iter::once(preferred).chain(DEFAULT_PORT + 1..DEFAULT_PORT + 100).find(|port| free(*port)).unwrap_or(0)
}

/// The local account is named after the person using the computer.
fn username() -> String {
    ["USERNAME", "USER"]
        .iter()
        .filter_map(|name| std::env::var(name).ok())
        .map(|name| name.trim().chars().take(64).collect::<String>())
        .find(|name| !name.is_empty())
        .unwrap_or_else(|| "admin".into())
}
