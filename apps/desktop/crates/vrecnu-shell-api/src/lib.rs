//! **The boundary.** Everything a shell may say to the engine, and everything the engine may
//! ask of a shell.
//!
//! This crate is why [ADR-006](../../../../docs/adr/006-phase-2-tauri-shell-with-swappable-core.md)
//! could reverse ADR-001 without betting the project on it. The Tauri shell, the headless CLI
//! shell, and a future `egui` shell all sit on the far side of these four traits. Swap the
//! shell and the engine does not notice.
//!
//! Two rules keep that true, and both are load-bearing:
//!
//! 1. **Nothing here may name a GUI framework.** Not in a type, not in a feature flag.
//! 2. **The engine never calls into a shell synchronously on a hot path.** Frames and samples
//!    go to sinks; the shell observes state through a coalesced event channel.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::path::{Path, PathBuf};

use crossbeam_channel::Receiver;
use serde::{Deserialize, Serialize};
use vrecnu_types::{
    AudioDevices, CameraInfo, CaptureSource, CaptureSourceId, CaptureStats, EditSpec, ErrorCode,
    PixelFormat, Recording, RenderId, SessionConfig, SessionId, SessionState,
};

/// The single error type a shell ever sees.
///
/// Deliberately flat and `Serialize`. Shells must not match on forty variants: they would all
/// need rewriting on a shell swap, and making the rich per-crate enums serializable would
/// force `serde` into every leaf crate. The engine performs the one conversion.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppError {
    /// Stable, greppable identifier with a `docs/errors.md` anchor.
    pub code: ErrorCode,
    /// One sentence, user-facing, no jargon.
    pub message: String,
    /// What the user can *do*. ADR-002 requires one for the all-ports-busy case.
    pub hint: Option<String>,
    /// Developer detail — the `Display` chain. Logged; collapsible in the UI.
    pub detail: Option<String>,
    /// Whether retrying the same action makes sense.
    pub recoverable: bool,
    /// Whether the user's recording was lost.
    ///
    /// Separate from `recoverable` on purpose: "the encoder died and your recording is gone"
    /// and "port 47821 is busy" are both recoverable actions, and must not render alike.
    pub session_lost: bool,
}

impl AppError {
    /// A recoverable error that cost the user nothing.
    #[must_use]
    pub fn recoverable(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: None,
            detail: None,
            recoverable: true,
            session_lost: false,
        }
    }

    /// An error that destroyed the recording. Shells should treat this as severe.
    #[must_use]
    pub fn fatal(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            hint: None,
            detail: None,
            recoverable: false,
            session_lost: true,
        }
    }

    /// Attaches the "what to do about it" line.
    #[must_use]
    pub fn with_hint(mut self, hint: impl Into<String>) -> Self {
        self.hint = Some(hint.into());
        self
    }

    /// Attaches the developer-facing cause chain.
    #[must_use]
    pub fn with_detail(mut self, detail: impl Into<String>) -> Self {
        self.detail = Some(detail.into());
        self
    }
}

impl std::fmt::Display for AppError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)?;
        if let Some(hint) = &self.hint {
            write!(f, " ({hint})")?;
        }
        Ok(())
    }
}

impl std::error::Error for AppError {}

/// Shorthand for fallible boundary calls.
pub type Result<T> = std::result::Result<T, AppError>;

/// A small preview image of a capture source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thumbnail {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
    /// Tightly packed BGRA8.
    pub bgra: Vec<u8>,
}

/// A complete read of engine state, for a shell that is starting cold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppState {
    /// Where the session is in its lifecycle.
    pub session: SessionState,
    /// Active session, if any.
    pub session_id: Option<SessionId>,
    /// Milliseconds recorded so far.
    pub elapsed_ms: u64,
    /// Live capture counters.
    pub stats: CaptureStats,
    /// The finished recording, once there is one.
    pub recording: Option<Recording>,
    /// The last error, if the session failed.
    pub error: Option<AppError>,
    /// Whether a publish session is currently serving.
    pub publishing: bool,
}

/// Everything the engine tells a shell about, after coalescing.
///
/// Nothing here is per-frame. `AudioLevels` is emitted at ~15 Hz, `Elapsed` at ~4 Hz, and
/// `CaptureStats` at ~1 Hz. Camera frames reach the screen through [`BubbleSurface::present`];
/// screen frames never leave the engine at all. A per-frame event across a WebView IPC
/// boundary would spend more CPU on notification than on encoding.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum EngineEvent {
    /// The session moved between lifecycle states.
    StateChanged {
        /// Previous state.
        from: SessionState,
        /// New state.
        to: SessionState,
    },
    /// Recording time, coalesced to ~4 Hz.
    Elapsed {
        /// Milliseconds since capture started.
        ms: u64,
    },
    /// Peak levels for the meters, coalesced to ~15 Hz.
    AudioLevels {
        /// Microphone peak in dBFS.
        mic_db: f32,
        /// System-audio peak in dBFS.
        system_db: f32,
    },
    /// Capture counters, coalesced to ~1 Hz.
    Stats(CaptureStats),
    /// Something is wrong but the recording continues.
    Warning {
        /// Stable identifier.
        code: ErrorCode,
        /// User-facing text.
        message: String,
    },
    /// Render progress, 0.0 to 1.0.
    RenderProgress {
        /// Which render.
        id: RenderId,
        /// Fraction complete.
        fraction: f32,
    },
    /// A render finished.
    RenderDone {
        /// Which render.
        id: RenderId,
        /// The rendered file.
        recording: Recording,
    },
    /// The publish page is being served.
    PublishOpened {
        /// URL including the capability token.
        url: String,
        /// The port that was actually bound.
        port: u16,
    },
    /// Upload progress reported by the publish page.
    PublishProgress {
        /// Bytes acknowledged by Drive.
        bytes_sent: u64,
        /// Total bytes.
        total: u64,
    },
    /// The upload succeeded and a share link exists.
    PublishDone {
        /// Drive's `webViewLink`.
        web_view_link: String,
        /// Drive file id.
        file_id: String,
    },
    /// The publish attempt failed.
    PublishFailed {
        /// Why.
        error: AppError,
    },
    /// The session failed.
    Failed {
        /// Why.
        error: AppError,
    },
}

/// Shell to engine. Implemented once, by the engine; shells hold `Arc<dyn RecorderEngine>`.
///
/// Every method is non-blocking or bounded-fast. Shells call these from their UI thread, so
/// nothing here may block on I/O for longer than a repaint.
pub trait RecorderEngine: Send + Sync + 'static {
    /// Screens and windows available to capture.
    fn list_capture_sources(&self) -> Result<Vec<CaptureSource>>;
    /// A preview image for the source picker.
    fn capture_thumbnail(&self, id: &CaptureSourceId, max_px: u32) -> Result<Thumbnail>;
    /// Cameras available for the bubble.
    fn list_cameras(&self) -> Result<Vec<CameraInfo>>;
    /// Microphones and loopback-capable outputs.
    fn list_audio_devices(&self) -> Result<AudioDevices>;

    /// Stores the configuration for the next session.
    fn configure(&self, config: SessionConfig) -> Result<()>;
    /// Opens devices and proves permissions, moving to `Ready` without recording yet.
    fn arm(&self) -> Result<()>;
    /// Begins capture.
    fn start(&self) -> Result<SessionId>;
    /// Ends capture and drains the encoder.
    fn stop(&self) -> Result<()>;
    /// Abandons the session and deletes its temporary files.
    fn cancel(&self) -> Result<()>;
    /// Returns to `Idle`, discarding any finished recording reference.
    fn reset(&self) -> Result<()>;

    /// Describes what an edit would produce without rendering it.
    fn preview_edit(&self, edit: &EditSpec) -> Result<EditPreview>;
    /// Renders an edit. Asynchronous; progress arrives as events.
    fn apply_edit(&self, edit: EditSpec) -> Result<RenderId>;
    /// Copies the finished recording somewhere the user chose.
    fn save_copy_to(&self, destination: &Path) -> Result<()>;

    /// Starts the loopback publish server and returns where to point a browser.
    fn begin_publish(&self) -> Result<PublishHandle>;
    /// Tears the publish server down early.
    fn abort_publish(&self) -> Result<()>;
    /// Delivers a share link obtained out-of-band.
    ///
    /// Only for shells that intercept the result themselves. The default path is
    /// `POST /api/complete` on the loopback server, which every shell gets for free — see
    /// ADR-006 on why the link must not come back over Tauri IPC.
    fn submit_publish_result(&self, result: PublishResult) -> Result<()>;

    /// A full state read, for a shell that just started or a webview that reloaded.
    fn snapshot(&self) -> AppState;
    /// Subscribes to the coalesced event stream.
    fn subscribe(&self) -> Receiver<EngineEvent>;
}

/// What an edit would produce, computed without rendering.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditPreview {
    /// Duration of the result.
    pub duration_ms: u64,
    /// Whether this can take FFmpeg's `-c copy` fast path.
    pub passthrough: bool,
}

/// Where the publish page is being served.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishHandle {
    /// Full URL including the capability token.
    pub url: String,
    /// The bound port, one of the five ADR-002 registers.
    pub port: u16,
}

/// The outcome the publish page reports.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishResult {
    /// Drive file id.
    pub file_id: String,
    /// Drive's `webViewLink`.
    pub web_view_link: String,
}

/// Engine to shell: things the engine cannot do for itself.
///
/// `ffmpeg_path` and `data_dir` exist specifically to keep a shell's path resolver out of the
/// engine. Reaching for `tauri::path` inside the engine is the most predictable way the
/// boundary would rot, so the engine simply cannot see it.
pub trait ShellServices: Send + Sync + 'static {
    /// Puts text on the clipboard — normally the share link.
    fn set_clipboard_text(&self, text: &str) -> Result<()>;
    /// Opens a URL in the user's browser.
    fn open_external_url(&self, url: &str) -> Result<()>;
    /// Reveals a file in Explorer/Finder.
    fn reveal_in_file_manager(&self, path: &Path) -> Result<()>;
    /// Surfaces a notification.
    fn notify(&self, level: NotifyLevel, title: &str, body: &str);
    /// Absolute path to the bundled `ffmpeg.exe`.
    fn ffmpeg_path(&self) -> Result<PathBuf>;
    /// Writable directory for recordings and temporary files.
    fn data_dir(&self) -> Result<PathBuf>;
}

/// Severity for [`ShellServices::notify`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NotifyLevel {
    /// Informational.
    Info,
    /// Something the user should know about but that is not fatal.
    Warning,
    /// A failure.
    Error,
}

/// Pixel layout a bubble implementation wants frames in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BubbleFrameFormat {
    /// Premultiplied BGRA — what `UpdateLayeredWindow` requires.
    Bgra8Premultiplied,
    /// Plain RGBA, for a canvas-based implementation.
    Rgba8,
    /// JPEG at the given quality, for an implementation behind an IPC boundary.
    Jpeg {
        /// 1–100.
        quality: u8,
    },
}

/// One camera frame, already scaled to the bubble's requested size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BubbleFrame {
    /// Frame width.
    pub width: u32,
    /// Frame height.
    pub height: u32,
    /// Encoded per [`BubbleSurface::preferred_frame_format`].
    pub data: Vec<u8>,
    /// The layout `data` is in.
    pub format: PixelFormat,
}

/// A surface that shows live camera frames on top of everything else.
///
/// The default implementation is a raw Win32 layered window, which has no shell dependency at
/// all and therefore survives a shell swap untouched. Keeping this a trait costs nothing and
/// leaves room for a webview-backed bubble if the M1 spike ever demands one.
pub trait BubbleSurface: Send + Sync + 'static {
    /// What encoding to hand to [`Self::present`].
    fn preferred_frame_format(&self) -> BubbleFrameFormat;
    /// Size the engine should scale frames to before presenting.
    fn preferred_size_px(&self) -> u32;
    /// Shows the bubble.
    fn show(&self, spec: vrecnu_types::BubbleSpec) -> Result<()>;
    /// Hides it without destroying it.
    fn hide(&self) -> Result<()>;
    /// Changes shape, size or position.
    fn update_spec(&self, spec: vrecnu_types::BubbleSpec) -> Result<()>;
    /// Displays a frame. Called on the camera thread; must return promptly.
    fn present(&self, frame: &BubbleFrame) -> Result<()>;
    /// Current on-screen rectangle — the user may have dragged it, and the burned-in overlay
    /// has to match what they see.
    fn geometry(&self) -> Option<vrecnu_types::Rect>;
}

/// How a shell shows the publish page.
pub trait PublishSurface: Send + Sync + 'static {
    /// Opens the loopback URL: an embedded webview, or the system browser.
    fn open(&self, url: &str, hint: PublishWindowHint) -> Result<()>;
    /// Closes it, if this shell owns a window.
    fn close(&self);
    /// Whether this shell can deliver the share link out-of-band.
    ///
    /// Defaults to `false`, and is expected to stay that way — see ADR-006.
    fn provides_ipc_result_channel(&self) -> bool {
        false
    }
}

/// Sizing hint for the publish window.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PublishWindowHint {
    /// Suggested width.
    pub width: u32,
    /// Suggested height.
    pub height: u32,
}

impl Default for PublishWindowHint {
    fn default() -> Self {
        Self {
            width: 520,
            height: 680,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fatal_errors_report_the_recording_as_lost() {
        let err = AppError::fatal(ErrorCode::EncodeDied, "The encoder stopped unexpectedly.");
        assert!(err.session_lost);
        assert!(!err.recoverable);
    }

    #[test]
    fn recoverable_errors_do_not_claim_data_loss() {
        // "port busy" must never render with the severity of "your recording is gone".
        let err =
            AppError::recoverable(ErrorCode::PublishPortsBusy, "All publish ports are in use.");
        assert!(!err.session_lost);
        assert!(err.recoverable);
    }

    #[test]
    fn display_includes_the_actionable_hint() {
        let err =
            AppError::recoverable(ErrorCode::PublishPortsBusy, "All publish ports are in use.")
                .with_hint("Close whatever is using ports 47821-47825.");
        assert_eq!(
            err.to_string(),
            "All publish ports are in use. (Close whatever is using ports 47821-47825.)"
        );
    }

    #[test]
    fn events_serialize_with_a_discriminating_tag() {
        let json = serde_json::to_string(&EngineEvent::Elapsed { ms: 1_500 }).unwrap();
        assert!(json.contains(r#""type":"elapsed""#), "got {json}");
        assert!(json.contains(r#""ms":1500"#), "got {json}");
    }

    #[test]
    fn publish_surfaces_default_to_no_ipc_channel() {
        struct Browser;
        impl PublishSurface for Browser {
            fn open(&self, _url: &str, _hint: PublishWindowHint) -> Result<()> {
                Ok(())
            }
            fn close(&self) {}
        }
        // The default matters: it is what keeps `packages/publish` free of a shell branch.
        assert!(!Browser.provides_ipc_result_channel());
    }
}
