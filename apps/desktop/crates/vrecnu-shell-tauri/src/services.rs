//! `ShellServices` for the Tauri shell. Everything here is called from Rust; the webview is
//! granted no clipboard, opener or notification permission (spec D4).

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;
use vrecnu_shell_api::{AppError, NotifyLevel, Result, ShellServices};
use vrecnu_types::ErrorCode;

/// Overrides FFmpeg discovery, as it does for `vrecnu-cli`.
const FFMPEG_ENV: &str = "VRECNU_FFMPEG";
const FFMPEG_FILE: &str = if cfg!(windows) {
    "ffmpeg.exe"
} else {
    "ffmpeg"
};
const RECORDINGS_DIR: &str = "recordings";

/// The shell services the engine calls back into.
pub(crate) struct TauriShell {
    app: AppHandle,
}

impl TauriShell {
    pub(crate) fn new(app: AppHandle) -> Self {
        Self { app }
    }
}

impl ShellServices for TauriShell {
    fn set_clipboard_text(&self, text: &str) -> Result<()> {
        self.app
            .clipboard()
            .write_text(text.to_owned())
            .map_err(|err| failure("vrecnu could not copy to the clipboard.", err))
    }

    fn open_external_url(&self, url: &str) -> Result<()> {
        self.app
            .opener()
            .open_url(url, None::<&str>)
            .map_err(|err| failure("vrecnu could not open your browser.", err))
    }

    fn reveal_in_file_manager(&self, path: &Path) -> Result<()> {
        self.app
            .opener()
            .reveal_item_in_dir(path)
            .map_err(|err| failure("vrecnu could not open File Explorer.", err))
    }

    fn notify(&self, level: NotifyLevel, title: &str, body: &str) {
        match level {
            NotifyLevel::Error => tracing::error!("{title}: {body}"),
            NotifyLevel::Warning => tracing::warn!("{title}: {body}"),
            NotifyLevel::Info => tracing::info!("{title}: {body}"),
        }
        if let Err(err) = self
            .app
            .notification()
            .builder()
            .title(title)
            .body(body)
            .show()
        {
            tracing::warn!(%err, "could not show a notification");
        }
    }

    /// `VRECNU_FFMPEG`, then the resource dir (M8's bundled sidecar), then next to the exe,
    /// then `PATH` - the same order as `CliShell`, so the two shells cannot disagree.
    fn ffmpeg_path(&self) -> Result<PathBuf> {
        let exe_dir = std::env::current_exe()
            .ok()
            .and_then(|exe| exe.parent().map(Path::to_path_buf));
        let dirs: Vec<PathBuf> = [self.app.path().resource_dir().ok(), exe_dir]
            .into_iter()
            .flatten()
            .collect();
        Ok(resolve_ffmpeg(std::env::var_os(FFMPEG_ENV), &dirs))
    }

    fn data_dir(&self) -> Result<PathBuf> {
        let base = self
            .app
            .path()
            .app_local_data_dir()
            .map_err(|err| failure("vrecnu could not find a place to save recordings.", err))?;
        ensure_recordings_dir(&base)
    }
}

/// Picks the FFmpeg to spawn. Pure, so the precedence is testable without an app.
pub(crate) fn resolve_ffmpeg(explicit: Option<OsString>, search_dirs: &[PathBuf]) -> PathBuf {
    if let Some(path) = explicit.filter(|value| !value.is_empty()) {
        return PathBuf::from(path);
    }
    search_dirs
        .iter()
        .map(|dir| dir.join(FFMPEG_FILE))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| PathBuf::from("ffmpeg"))
}

/// `<base>/recordings`, created on first use.
pub(crate) fn ensure_recordings_dir(base: &Path) -> Result<PathBuf> {
    let dir = base.join(RECORDINGS_DIR);
    std::fs::create_dir_all(&dir)
        .map_err(|err| failure("vrecnu could not create its recordings folder.", err))?;
    Ok(dir)
}

/// Shell failures cost the user nothing already recorded: recoverable, never `session_lost`.
fn failure(message: &str, err: impl std::fmt::Display) -> AppError {
    AppError::recoverable(ErrorCode::Internal, message).with_detail(err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_with_ffmpeg() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(FFMPEG_FILE), b"").unwrap();
        dir
    }

    #[test]
    fn an_explicit_override_wins_over_a_bundled_copy() {
        let bundled = dir_with_ffmpeg();
        let found = resolve_ffmpeg(Some("C:/x/ffmpeg.exe".into()), &[bundled.path().into()]);
        assert_eq!(found, PathBuf::from("C:/x/ffmpeg.exe"));
    }

    #[test]
    fn an_empty_override_is_ignored_rather_than_spawning_an_empty_path() {
        assert_eq!(
            resolve_ffmpeg(Some(OsString::new()), &[]),
            PathBuf::from("ffmpeg")
        );
    }

    #[test]
    fn the_first_directory_holding_ffmpeg_wins() {
        let empty = tempfile::tempdir().unwrap();
        let (first, second) = (dir_with_ffmpeg(), dir_with_ffmpeg());
        let dirs = [
            empty.path().into(),
            first.path().into(),
            second.path().into(),
        ];
        assert_eq!(resolve_ffmpeg(None, &dirs), first.path().join(FFMPEG_FILE));
    }

    #[test]
    fn with_nothing_bundled_the_bare_name_is_left_for_path_lookup() {
        // Same contract as CliShell: the spawn failure names the path, not a guess made here.
        let empty = tempfile::tempdir().unwrap();
        assert_eq!(
            resolve_ffmpeg(None, &[empty.path().into()]),
            PathBuf::from("ffmpeg")
        );
    }

    #[test]
    fn the_recordings_folder_is_created_inside_the_data_directory_and_reused() {
        let base = tempfile::tempdir().unwrap();
        let dir = ensure_recordings_dir(base.path()).unwrap();
        assert_eq!(dir, base.path().join(RECORDINGS_DIR));
        assert!(dir.is_dir());
        assert_eq!(ensure_recordings_dir(base.path()).unwrap(), dir);
    }

    #[test]
    fn an_unusable_data_directory_is_recoverable_and_loses_nothing() {
        let base = tempfile::tempdir().unwrap();
        let file = base.path().join("not-a-dir");
        std::fs::write(&file, b"").unwrap();
        let err = ensure_recordings_dir(&file).unwrap_err();
        assert!(err.recoverable);
        assert!(!err.session_lost);
    }
}
