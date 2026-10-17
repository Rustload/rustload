//! Asks the person whether the browser extension may connect: it then receives the interface's
//! address and the API key (`POST /api/desktop/pair`), instead of having them typed in by hand.

use rustload::{Handle, PairRequest};
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::{DialogExt, MessageDialogButtons, MessageDialogKind};
use tokio::sync::oneshot;

use crate::{WINDOW, text::text};

/// Lets the server running in the app ask the question (local mode only: in gateway mode the
/// extension connects to the server itself).
pub fn register(app: &AppHandle, handle: &Handle) {
    let app = app.clone();
    handle.set_pairing_approver(move |request: PairRequest| {
        let app = app.clone();
        async move {
            let (answer, answered) = oneshot::channel();
            let t = text();
            let mut dialog = app
                .dialog()
                .message(t.pair_question(&request.browser))
                .title("Rustload")
                .kind(MessageDialogKind::Info)
                .buttons(MessageDialogButtons::OkCancelCustom(t.pair_allow.into(), t.pair_refuse.into()));
            // On top of the interface when it is open, which comes to the front.
            if let Some(window) = app.get_webview_window(WINDOW) {
                let _ = window.unminimize();
                let _ = window.set_focus();
                dialog = dialog.parent(&window);
            }
            dialog.show(move |allowed| {
                let _ = answer.send(allowed);
            });
            answered.await.unwrap_or(false)
        }
    });
}
