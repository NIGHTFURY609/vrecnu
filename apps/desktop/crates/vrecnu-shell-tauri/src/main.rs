//! vrecnu's desktop shell (M6).
//!
//! **This is the only crate allowed to name `tauri`, `tao`, `wry` or `webview2-com`**
//! (ADR-006). Tauri is an output device and an input device here, not the application: the
//! engine runs the same with `vrecnu-cli` driving it, and `check-shell-boundary.ps1` keeps it so.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

mod tray;
mod window;

use tauri::RunEvent;

fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("VRECNU_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let app = tauri::Builder::default()
        // First, so a second launch is turned away before it builds anything of its own: two
        // instances would mean two tray icons and two publish servers racing for the same ports.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::open_main(app);
        }))
        .setup(|app| {
            tray::build(app.handle())?;
            Ok(())
        })
        .build(tauri::generate_context!());

    let app = match app {
        Ok(app) => app,
        Err(err) => {
            tracing::error!(%err, "vrecnu could not start");
            std::process::exit(1);
        }
    };

    app.run(|_app, event| {
        // Closing the last window raises ExitRequested with no code; the tray keeps the app
        // alive. Quit calls `exit(0)`, which carries a code and is let through.
        if let RunEvent::ExitRequested {
            code: None, api, ..
        } = event
        {
            api.prevent_exit();
        }
    });
}
