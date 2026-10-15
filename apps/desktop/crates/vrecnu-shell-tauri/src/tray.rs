//! The tray icon: the app's only permanent surface (ADR-006's RSS mitigation).
//!
//! The menu acts on the engine directly, so a recording can be started and stopped with no
//! window open. What the tray *shows* is driven by engine events, in `lifecycle`.

use std::sync::Arc;
use std::time::Duration;

use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Wry};
use vrecnu_engine::Session;
use vrecnu_shell_api::{NotifyLevel, RecorderEngine, ShellServices};

use crate::lifecycle::{self, RecordAction, TraySurface, TrayView, WindowAction};
use crate::services::TauriShell;
use crate::window;

const TRAY_ID: &str = "vrecnu";
const OPEN: &str = "open";
const RECORD: &str = "record";
const QUIT: &str = "quit";

/// How long Quit waits for a recording to finish saving before exiting anyway. Matches the
/// engine's cancel grace; FFmpeg's drain is bounded well inside it.
const QUIT_GRACE: Duration = Duration::from_secs(20);
const QUIT_POLL: Duration = Duration::from_millis(100);

/// Installs the tray icon and its menu. Returns the Start/Stop item, which [`AppTray`]
/// relabels as the engine moves.
pub(crate) fn build(app: &AppHandle) -> tauri::Result<MenuItem<Wry>> {
    let open = MenuItem::with_id(app, OPEN, "Open vrecnu", true, None::<&str>)?;
    let record = MenuItem::with_id(app, RECORD, lifecycle::START_LABEL, true, None::<&str>)?;
    let quit = MenuItem::with_id(app, QUIT, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &record, &quit])?;

    let mut tray = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("vrecnu")
        .menu(&menu)
        // Left click opens the window; the menu is for right click.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            OPEN => window::open_main(app),
            RECORD => toggle_recording(app),
            QUIT => quit_app(app),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                window::open_main(tray.app_handle());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(record)
}

fn session(app: &AppHandle) -> Arc<Session> {
    Arc::clone(app.state::<Arc<Session>>().inner())
}

/// Starts with whatever the window last configured (the engine holds it), or the defaults if
/// the window was never opened.
fn toggle_recording(app: &AppHandle) {
    let session = session(app);
    let result = match lifecycle::record_action(session.snapshot().session) {
        RecordAction::Start => vrecnu_engine::spawn_session(&session).map(|_| ()),
        RecordAction::Stop => RecorderEngine::stop(session.as_ref()),
        RecordAction::Nothing => Ok(()),
    };
    // With the window closed, a notification is the only place this error can be seen.
    if let Err(err) = result {
        app.state::<Arc<TauriShell>>().notify(
            NotifyLevel::Error,
            "vrecnu could not record",
            &err.to_string(),
        );
    }
}

/// Quit during a recording stops it and lets FFmpeg finish the file first, so quitting never
/// leaves a half-written recording. The wait runs off the event loop, so the tray stays
/// responsive while it lasts.
fn quit_app(app: &AppHandle) {
    let session = session(app);
    if !lifecycle::is_capturing(session.snapshot().session) {
        app.exit(0);
        return;
    }
    if let Err(err) = RecorderEngine::stop(session.as_ref()) {
        tracing::warn!(%err, "could not stop the recording before quitting");
    }
    let handle = app.clone();
    let waiter = std::thread::Builder::new()
        .name("vrecnu-quit".into())
        .spawn(move || {
            let settled = lifecycle::wait_until(QUIT_GRACE, QUIT_POLL, || {
                !lifecycle::is_capturing(session.snapshot().session)
            });
            if !settled {
                tracing::warn!("quitting before the recording finished saving");
            }
            handle.exit(0);
        });
    if let Err(err) = waiter {
        tracing::warn!(%err, "could not wait for the recording to save");
        app.exit(0);
    }
}

/// The real [`TraySurface`]: the tooltip, the Start/Stop item and the main window. Tauri runs
/// these setters on the main thread itself, so calling them from the observer thread is safe.
pub(crate) struct AppTray {
    app: AppHandle,
    record: MenuItem<Wry>,
}

impl AppTray {
    pub(crate) fn new(app: AppHandle, record: MenuItem<Wry>) -> Self {
        Self { app, record }
    }
}

impl TraySurface for AppTray {
    fn show(&self, view: &TrayView) {
        let tooltip = self
            .app
            .tray_by_id(TRAY_ID)
            .map_or(Ok(()), |tray| tray.set_tooltip(Some(&view.tooltip)));
        for result in [
            tooltip,
            self.record.set_text(view.record_label),
            self.record.set_enabled(view.record_enabled),
        ] {
            if let Err(err) = result {
                tracing::warn!(%err, "could not update the tray");
            }
        }
    }

    fn window(&self, action: WindowAction) {
        match action {
            WindowAction::Open => window::open_main(&self.app),
            WindowAction::Close => window::close_main(&self.app),
        }
    }
}
