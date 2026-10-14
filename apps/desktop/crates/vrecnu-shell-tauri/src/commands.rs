//! Thin IPC wrappers over `Arc<Session>`.
//!
//! All `async`: Tauri runs them on its worker pool, so a slow device enumeration (Media
//! Foundation, WASAPI) never blocks the event loop that drives the tray and the window.
//! `AppError` is `Serialize`, so `hint` and `session_lost` reach the UI intact.

use std::sync::Arc;

use tauri::State;
use vrecnu_engine::Session;
use vrecnu_shell_api::{AppError, AppState, RecorderEngine};
use vrecnu_types::{AudioDevices, CameraInfo, CaptureSource};

type Engine<'a> = State<'a, Arc<Session>>;

/// The UI calls this first on every window creation.
#[tauri::command(async)]
pub(crate) fn get_snapshot(engine: Engine<'_>) -> Result<AppState, AppError> {
    Ok(engine.snapshot())
}

#[tauri::command(async)]
pub(crate) fn list_sources(engine: Engine<'_>) -> Result<Vec<CaptureSource>, AppError> {
    engine.list_capture_sources()
}

#[tauri::command(async)]
pub(crate) fn list_cameras(engine: Engine<'_>) -> Result<Vec<CameraInfo>, AppError> {
    engine.list_cameras()
}

#[tauri::command(async)]
pub(crate) fn list_audio_devices(engine: Engine<'_>) -> Result<AudioDevices, AppError> {
    engine.list_audio_devices()
}
