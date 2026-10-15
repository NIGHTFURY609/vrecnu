//! Thin IPC wrappers over `Arc<Session>`.
//!
//! All `async`: Tauri runs them on its worker pool, so a slow device enumeration (Media
//! Foundation, WASAPI) never blocks the event loop that drives the tray and the window.
//! `AppError` is `Serialize`, so `hint` and `session_lost` reach the UI intact.

use std::path::PathBuf;
use std::sync::Arc;

use tauri::State;
use vrecnu_engine::Session;
use vrecnu_shell_api::{AppError, AppState, RecorderEngine, ShellServices};
use vrecnu_types::{
    AudioDevices, CameraInfo, CaptureSource, ErrorCode, SessionConfig, SessionId,
};

use crate::services::TauriShell;

type Engine<'a> = State<'a, Arc<Session>>;
type Shell<'a> = State<'a, Arc<TauriShell>>;

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

/// What the next recording will use. The window shows this rather than its own defaults, so
/// it and the tray's "Start recording" agree.
#[tauri::command(async)]
pub(crate) fn get_config(engine: Engine<'_>) -> Result<SessionConfig, AppError> {
    Ok(engine.config())
}

/// The window calls this on every change, not only at Record, for the same reason.
#[tauri::command(async)]
pub(crate) fn configure(engine: Engine<'_>, config: SessionConfig) -> Result<(), AppError> {
    engine.configure(config)
}

/// `spawn_session`, not `start`: `start` only moves the state machine, and the worker that
/// records needs the `Arc<Session>` (see `vrecnu_engine::spawn_session`).
#[tauri::command(async)]
pub(crate) fn start_recording(engine: Engine<'_>) -> Result<SessionId, AppError> {
    vrecnu_engine::spawn_session(engine.inner())
}

/// Returns at once; `Stopped` arrives as an event once FFmpeg has finished the file.
#[tauri::command(async)]
pub(crate) fn stop_recording(engine: Engine<'_>) -> Result<(), AppError> {
    engine.stop()
}

/// Back to Idle. Drops the engine's reference to the last recording; the file stays.
#[tauri::command(async)]
pub(crate) fn reset(engine: Engine<'_>) -> Result<(), AppError> {
    engine.reset()
}

/// Opens Explorer on the finished recording. The opener is used from Rust; the webview holds
/// no opener permission (spec D4).
#[tauri::command(async)]
pub(crate) fn reveal_recording(engine: Engine<'_>, shell: Shell<'_>) -> Result<(), AppError> {
    let path = recording_path(&engine.snapshot())?;
    shell.reveal_in_file_manager(&path)
}

/// The finished recording's path, or the error the UI shows when there is none.
fn recording_path(state: &AppState) -> Result<PathBuf, AppError> {
    state
        .recording
        .as_ref()
        .map(|recording| recording.path.clone())
        .ok_or_else(|| {
            AppError::recoverable(ErrorCode::IllegalState, "There is no recording to show yet.")
        })
}

#[cfg(test)]
mod tests {
    use vrecnu_types::{CaptureStats, Recording, SessionState};

    use super::*;

    fn state(recording: Option<Recording>) -> AppState {
        AppState {
            session: SessionState::Stopped,
            session_id: None,
            elapsed_ms: 0,
            stats: CaptureStats::default(),
            recording,
            error: None,
            publishing: false,
        }
    }

    #[test]
    fn revealing_with_no_recording_is_an_illegal_state_not_a_crash() {
        let err = recording_path(&state(None)).unwrap_err();
        assert_eq!(err.code, ErrorCode::IllegalState);
        assert!(!err.session_lost, "there was no recording to lose");
    }

    #[test]
    fn revealing_uses_the_finished_recordings_path() {
        let recording = Recording {
            session: SessionId(1),
            path: PathBuf::from("C:/recordings/vrecnu-1.mp4"),
            duration_ms: 1,
            size_bytes: 1,
            mime_type: "video/mp4".into(),
        };
        assert_eq!(
            recording_path(&state(Some(recording))).unwrap(),
            PathBuf::from("C:/recordings/vrecnu-1.mp4")
        );
    }
}
