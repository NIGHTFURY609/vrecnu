//! vrecnu's desktop shell (M6).
//!
//! **This is the only crate allowed to name `tauri`, `tao`, `wry` or `webview2-com`**
//! (ADR-006). Tauri is an output device and an input device here, not the application: the
//! engine runs the same with `vrecnu-cli` driving it, and `check-shell-boundary.ps1` keeps it so.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
#![forbid(unsafe_code)]

mod commands;
#[cfg(test)]
mod contract_tests;
mod events;
mod lifecycle;
mod services;
mod tray;
mod window;

use std::sync::Arc;

use tauri::{Manager, RunEvent};
use vrecnu_engine::Session;
use vrecnu_shell_api::RecorderEngine;

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
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let shell = Arc::new(services::TauriShell::new(app.handle().clone()));
            let session = Arc::new(Session::new(shell.clone()));
            // Both subscriptions before anything can emit; the UI recovers older state from
            // get_snapshot, not from replayed events.
            let engine_events = RecorderEngine::subscribe(session.as_ref());
            let tray_events = RecorderEngine::subscribe(session.as_ref());
            events::spawn_pump(
                engine_events,
                events::WebviewSink::new(app.handle().clone()),
            )?;
            let initial = session.snapshot().session;
            // Managed before the tray exists: its menu handlers read both.
            app.manage(session);
            app.manage(shell);
            let record_item = tray::build(app.handle())?;
            lifecycle::spawn_observer(
                tray_events,
                initial,
                tray::AppTray::new(app.handle().clone(), record_item),
            )?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::get_snapshot,
            commands::list_sources,
            commands::list_cameras,
            commands::list_audio_devices,
            commands::get_config,
            commands::configure,
            commands::start_recording,
            commands::stop_recording,
            commands::reset,
            commands::reveal_recording
        ])
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
