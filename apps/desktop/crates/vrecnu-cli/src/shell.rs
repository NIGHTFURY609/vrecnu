//! The headless shell.
//!
//! Implementing [`ShellServices`] for real, rather than passing paths around as strings, is what
//! makes this binary evidence rather than a demo: it exercises the same trait the Tauri shell
//! will, so a method that quietly grows a `tauri::` dependency stops compiling here first
//! (ADR-006).

use std::path::{Path, PathBuf};

use vrecnu_shell_api::{AppError, NotifyLevel, Result, ShellServices};
use vrecnu_types::ErrorCode;

/// Overrides FFmpeg discovery. M8 ships a stripped build alongside the exe; until then this is
/// how a developer points at a local one.
const FFMPEG_ENV: &str = "VRECNU_FFMPEG";

/// A shell with no window, no clipboard and no notifications.
#[derive(Debug, Default)]
pub struct CliShell;

impl CliShell {
    /// Locates `ffmpeg`, preferring the bundled copy over whatever is on `PATH`.
    ///
    /// The order is deliberate. A build that ships its own FFmpeg must use *that* one: ADR-004
    /// pins the version precisely because `-progress` output and encoder availability differ
    /// between builds, and silently falling through to a system FFmpeg would make bug reports
    /// unreproducible.
    fn locate_ffmpeg() -> Option<PathBuf> {
        if let Some(explicit) = std::env::var_os(FFMPEG_ENV) {
            return Some(PathBuf::from(explicit));
        }
        let bundled = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf))
            .map(|dir| {
                dir.join(if cfg!(windows) {
                    "ffmpeg.exe"
                } else {
                    "ffmpeg"
                })
            });
        match bundled {
            Some(path) if path.is_file() => Some(path),
            // Bare name, resolved by the OS against `PATH` at spawn time. Only reachable in a
            // development tree, which is exactly where it is wanted.
            _ => Some(PathBuf::from("ffmpeg")),
        }
    }
}

impl ShellServices for CliShell {
    fn set_clipboard_text(&self, _text: &str) -> Result<()> {
        Err(unsupported("copy to the clipboard"))
    }

    fn open_external_url(&self, _url: &str) -> Result<()> {
        Err(unsupported("open a browser"))
    }

    fn reveal_in_file_manager(&self, _path: &Path) -> Result<()> {
        Err(unsupported("open a file manager"))
    }

    fn notify(&self, level: NotifyLevel, title: &str, body: &str) {
        match level {
            NotifyLevel::Error => tracing::error!("{title}: {body}"),
            NotifyLevel::Warning => tracing::warn!("{title}: {body}"),
            NotifyLevel::Info => tracing::info!("{title}: {body}"),
        }
    }

    fn ffmpeg_path(&self) -> Result<PathBuf> {
        Self::locate_ffmpeg().ok_or_else(|| {
            AppError::fatal(
                ErrorCode::EncodeMissingBinary,
                "vrecnu could not find ffmpeg.",
            )
            .with_hint(format!("Set {FFMPEG_ENV} to the path of an ffmpeg binary."))
        })
    }

    fn data_dir(&self) -> Result<PathBuf> {
        // The working directory, not `%LOCALAPPDATA%`: a headless run writes where it was
        // invoked, so a developer finds the file without hunting for it.
        std::env::current_dir().map_err(|err| {
            AppError::recoverable(
                ErrorCode::Internal,
                "vrecnu could not find a place to write.",
            )
            .with_detail(err.to_string())
        })
    }
}

/// The CLI genuinely cannot do these; saying so beats pretending they succeeded.
fn unsupported(what: &str) -> AppError {
    AppError::recoverable(
        ErrorCode::Internal,
        format!("This build has no window and cannot {what}."),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_explicit_override_wins() {
        // Serial-unsafe by nature; kept to one test so no other test races it.
        std::env::set_var(FFMPEG_ENV, "C:/somewhere/ffmpeg.exe");
        let path = CliShell::locate_ffmpeg().expect("a path");
        std::env::remove_var(FFMPEG_ENV);
        assert_eq!(path, PathBuf::from("C:/somewhere/ffmpeg.exe"));
    }

    #[test]
    fn a_path_is_always_produced_so_the_failure_comes_from_spawning() {
        // Returning `None` here would report "missing binary" before anything was tried; letting
        // the spawn fail names the path that was attempted.
        assert!(CliShell.ffmpeg_path().is_ok());
    }

    #[test]
    fn unsupported_capabilities_are_recoverable_rather_than_fatal() {
        // Having no clipboard must never render as "your recording is gone".
        let err = CliShell.set_clipboard_text("x").unwrap_err();
        assert!(err.recoverable);
        assert!(!err.session_lost);
    }

    #[test]
    fn the_data_directory_is_where_the_command_was_run() {
        assert_eq!(
            CliShell.data_dir().unwrap(),
            std::env::current_dir().unwrap()
        );
    }
}
