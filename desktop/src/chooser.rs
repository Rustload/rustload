//! The page that chooses how the app is used, on first start and from the tray menu: Rustload on
//! this computer, or a gateway to a Rustload server.

use serde::Serialize;
use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

use crate::{
    Desktop,
    backend::{self, CheckError},
    prefs::Mode,
};

const WINDOW: &str = "chooser";

pub fn open(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(WINDOW) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return;
    }
    let _ = WebviewWindowBuilder::new(app, WINDOW, WebviewUrl::App("index.html".into()))
        .title("Rustload")
        .inner_size(560.0, 660.0)
        .resizable(false)
        .center()
        .build();
}

/// The mode in use, to start the page from (never the API key).
#[derive(Serialize)]
pub struct Current {
    mode: Option<&'static str>,
    url: String,
}

#[tauri::command]
pub fn current(app: AppHandle) -> Current {
    let desktop = app.state::<Desktop>();
    let prefs = desktop.prefs.lock().expect("poisoned");
    match &prefs.mode {
        None => Current { mode: None, url: String::new() },
        Some(Mode::Local) => Current { mode: Some("local"), url: String::new() },
        Some(Mode::Server { url, .. }) => Current { mode: Some("server"), url: url.clone() },
    }
}

#[tauri::command]
pub fn use_local(app: AppHandle) {
    switch(&app, Mode::Local);
}

/// Checks the server before using it. The error is a code the page words.
#[tauri::command]
pub async fn use_server(app: AppHandle, url: String, api_key: String) -> Result<(), &'static str> {
    let (base, _version) = backend::check(&url, &api_key).await.map_err(|error| match error {
        CheckError::Unreachable => "unreachable",
        CheckError::NotRustload => "notRustload",
        CheckError::BadKey => "badKey",
    })?;
    switch(&app, Mode::Server { url: base.to_string(), api_key: api_key.trim().to_owned() });
    Ok(())
}

/// Saves the mode and restarts the app in it.
fn switch(app: &AppHandle, mode: Mode) {
    let desktop = app.state::<Desktop>();
    let mut prefs = desktop.prefs.lock().expect("poisoned");
    prefs.mode = Some(mode);
    prefs.save(&desktop.data_dir);
    drop(prefs);
    app.request_restart();
}
