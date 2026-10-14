//! The one window. Created on demand and destroyed on close, never hidden: a hidden WebView2
//! keeps its renderer processes alive, which is the RSS ADR-006 is worried about (M9).

use tauri::{AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

/// Label of the main window. Commands and capabilities refer to it by this name.
pub(crate) const MAIN: &str = "main";

/// Focuses the main window, creating it if it does not exist.
pub(crate) fn open_main(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        // Already open: focus it rather than stacking a duplicate.
        for result in [window.unminimize(), window.show(), window.set_focus()] {
            if let Err(err) = result {
                tracing::warn!(%err, "could not bring the window forward");
            }
        }
        return;
    }
    let built = WebviewWindowBuilder::new(app, MAIN, WebviewUrl::App("index.html".into()))
        .title("vrecnu")
        .inner_size(400.0, 560.0)
        .resizable(false)
        .center()
        .build();
    if let Err(err) = built {
        tracing::error!(%err, "could not open the window");
    }
}
