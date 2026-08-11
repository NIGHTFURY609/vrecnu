//! Domain vocabulary for the vrecnu desktop app.
//!
//! This crate has **no platform dependencies** — no `windows`, no `tauri`, no I/O. It is the
//! one crate every other crate may depend on, which only stays true if it stays inert.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Identifies one recording session for its whole lifetime, including its temp directory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionId(pub u64);

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:016x}", self.0)
    }
}

/// Identifies an in-progress render (a trim, a filter pass, an export).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RenderId(pub u64);

/// Opaque handle to something the OS can capture.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CaptureSourceId(pub String);

/// What a capture source actually is. Mirrors `CaptureSourceKind` on the web side.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CaptureSourceKind {
    /// An entire display.
    Monitor,
    /// A single application window.
    Window,
}

/// A capturable target as offered to the user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureSource {
    /// Stable handle used to start capture.
    pub id: CaptureSourceId,
    /// Human-readable name. Never logged at info level — see the redaction note in the engine.
    pub title: String,
    /// Whether this is a monitor or a window.
    pub kind: CaptureSourceKind,
    /// Native pixel size.
    pub size: Size,
    /// True for the display the OS considers primary.
    pub is_primary: bool,
}

/// Width and height in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

impl Size {
    /// Bytes needed for one tightly-packed BGRA8 frame of this size.
    ///
    /// Tightly packed — a real capture surface is row-pitch aligned and needs `stride`, not
    /// this. Used for buffer-pool sizing, never for indexing into a mapped texture.
    #[must_use]
    pub const fn bgra_byte_len(self) -> usize {
        (self.width as usize) * (self.height as usize) * 4
    }
}

/// Centre-crop offsets and side length for fitting a frame into a square.
///
/// Lives here because both the camera pipeline and the bubble renderer need the identical
/// answer: if they disagreed by a pixel, the burned-in overlay would not match what the user
/// saw floating on screen.
#[must_use]
pub const fn square_crop(width: u32, height: u32) -> (u32, u32, u32) {
    let side = if width < height { width } else { height };
    ((width - side) / 2, (height - side) / 2, side)
}

/// A rectangle in screen coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    /// Left edge.
    pub x: i32,
    /// Top edge.
    pub y: i32,
    /// Width in pixels.
    pub width: u32,
    /// Height in pixels.
    pub height: u32,
}

/// Pixel layouts vrecnu actually moves between components.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PixelFormat {
    /// What Windows Graphics Capture produces: `DXGI_FORMAT_B8G8R8A8_UNORM`.
    Bgra8,
    /// Premultiplied BGRA, required by `UpdateLayeredWindow` for the bubble window.
    Bgra8Premultiplied,
    /// What the encoder wants; produced by FFmpeg's own conversion, not by us.
    Nv12,
}

/// An available camera.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CameraInfo {
    /// Stable device handle.
    pub id: String,
    /// Human-readable device name.
    pub name: String,
}

/// Audio devices offered to the user.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevices {
    /// Capture devices (microphones).
    pub inputs: Vec<AudioDevice>,
    /// Render devices, captured via WASAPI loopback for "system audio".
    pub outputs: Vec<AudioDevice>,
}

/// One audio endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    /// Stable device handle.
    pub id: String,
    /// Human-readable device name.
    pub name: String,
    /// True for the endpoint the OS currently defaults to.
    pub is_default: bool,
}

/// Everything needed to start a recording.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionConfig {
    /// The screen or window to capture. `None` records the camera only.
    pub source: Option<CaptureSourceId>,
    /// Camera to use, for the bubble and the burned-in overlay.
    pub camera: Option<String>,
    /// Microphone to mix in.
    pub microphone: Option<String>,
    /// Whether to capture system audio via WASAPI loopback.
    pub system_audio: bool,
    /// Target frame rate. 30 is the budgeted reference point (AGENTS.md §1).
    pub fps: u32,
    /// Target video bitrate in bits per second.
    pub video_bitrate: u32,
    /// Where the floating camera bubble sits, if it is shown.
    pub bubble: Option<BubbleSpec>,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            source: None,
            camera: None,
            microphone: None,
            system_audio: true,
            fps: 30,
            video_bitrate: 6_000_000,
            bubble: None,
        }
    }
}

/// Shape and placement of the floating camera bubble.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BubbleSpec {
    /// Diameter in physical pixels.
    pub diameter: u32,
    /// Top-left position in screen coordinates.
    pub position: (i32, i32),
    /// Whether the bubble ignores mouse events when idle.
    pub click_through: bool,
}

impl Default for BubbleSpec {
    fn default() -> Self {
        Self {
            diameter: 240,
            position: (48, 48),
            click_through: true,
        }
    }
}

/// A non-destructive edit. Applying it renders a new file; the capture is never overwritten.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EditSpec {
    /// Trim start, milliseconds from the beginning.
    pub start_ms: Option<u64>,
    /// Trim end, milliseconds from the beginning.
    pub end_ms: Option<u64>,
    /// `eq` filter adjustments.
    pub video: VideoAdjust,
    /// Loudness and filtering adjustments.
    pub audio: AudioAdjust,
}

impl EditSpec {
    /// True when nothing would change, so the render can take FFmpeg's `-c copy` fast path.
    #[must_use]
    pub fn is_passthrough(&self) -> bool {
        self.start_ms.unwrap_or(0) == 0
            && self.end_ms.is_none()
            && self.video.is_identity()
            && self.audio.is_identity()
    }
}

/// Maps directly onto FFmpeg's `eq` filter.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VideoAdjust {
    /// `eq=brightness`, -1.0 to 1.0.
    pub brightness: f32,
    /// `eq=contrast`, 1.0 is unchanged.
    pub contrast: f32,
    /// `eq=saturation`, 1.0 is unchanged.
    pub saturation: f32,
}

impl Default for VideoAdjust {
    fn default() -> Self {
        Self {
            brightness: 0.0,
            contrast: 1.0,
            saturation: 1.0,
        }
    }
}

impl VideoAdjust {
    /// True when this would produce a bit-identical image.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.brightness == 0.0 && self.contrast == 1.0 && self.saturation == 1.0
    }
}

/// Loudness and cleanup, mapping onto `volume`, `loudnorm` and `highpass`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioAdjust {
    /// Linear gain applied via `volume`.
    pub gain: f32,
    /// Whether to run `loudnorm`.
    pub normalize: bool,
    /// Whether to run a `highpass` rumble filter.
    pub denoise: bool,
}

impl Default for AudioAdjust {
    fn default() -> Self {
        Self {
            gain: 1.0,
            normalize: false,
            denoise: false,
        }
    }
}

impl AudioAdjust {
    /// True when this would leave the audio untouched.
    #[must_use]
    pub fn is_identity(&self) -> bool {
        self.gain == 1.0 && !self.normalize && !self.denoise
    }
}

/// Where a session is in its lifecycle. Deliberately mirrors `RecorderState` in
/// `packages/core` so the two halves of the product describe themselves the same way.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionState {
    /// Nothing configured yet.
    Idle,
    /// Devices are open and permissions proven; ready to start.
    Ready,
    /// Capturing.
    Recording,
    /// Capture stopped; encoder still draining.
    Finalizing,
    /// A finished recording is on disk.
    Stopped,
    /// Something failed. Carries no payload — the error travels separately.
    Failed,
}

/// A finished recording.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Recording {
    /// Session that produced it.
    pub session: SessionId,
    /// Absolute path on disk.
    pub path: PathBuf,
    /// Duration in milliseconds.
    pub duration_ms: u64,
    /// File size in bytes.
    pub size_bytes: u64,
    /// MIME type, for the publish page's upload metadata.
    pub mime_type: String,
}

/// Live capture counters. `frames_dropped` is user-visible and asserted in CI, because
/// AGENTS.md §1 budgets zero dropped frames at 1080p30 and a silent drop is indistinguishable
/// from success without it.
// No `Eq`: `encoder_fps` is an `f32`, and floats have no total equality. Deriving it does not
// compile, which is the correct outcome — a stats struct that claims total equality would be
// a lie the moment a NaN appeared.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaptureStats {
    /// Frames delivered by the OS capture callback.
    pub frames_in: u64,
    /// Frames handed to the encoder, including duplicates inserted by the pacer.
    pub frames_written: u64,
    /// Frames discarded because the encoder fell behind. Must stay at zero.
    pub frames_dropped: u64,
    /// Frames the pacer duplicated because the screen was static. Expected, not a fault.
    pub frames_duplicated: u64,
    /// Encoder throughput as reported by FFmpeg's `-progress` output.
    pub encoder_fps: f32,
}

/// Stable, greppable error identifiers. Each one has an anchor in `docs/errors.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[non_exhaustive]
pub enum ErrorCode {
    /// Windows Graphics Capture is unavailable on this OS build.
    CaptureUnavailable,
    /// The user declined or revoked screen-capture permission.
    CapturePermission,
    /// The captured window or monitor disappeared.
    CaptureSourceGone,
    /// The encoder fell behind and frames were discarded.
    CaptureBackpressure,
    /// WASAPI loopback could not be opened.
    AudioNoLoopback,
    /// An audio device disappeared mid-recording.
    AudioDeviceLost,
    /// The camera is held exclusively by another application.
    CameraBusy,
    /// No camera is present.
    CameraNone,
    /// Camera access was denied.
    CameraPermission,
    /// `ffmpeg.exe` is missing from the install directory.
    EncodeMissingBinary,
    /// FFmpeg exited with a failure status.
    EncodeFailed,
    /// FFmpeg died unexpectedly.
    EncodeDied,
    /// All five registered loopback ports are in use (ADR-002).
    PublishPortsBusy,
    /// The publish page assets were not embedded in the binary.
    PublishAssetsMissing,
    /// The publish session timed out.
    PublishTimeout,
    /// The publish page reported a failure.
    PublishPageError,
    /// Not enough disk space to continue.
    DiskFull,
    /// The requested action is not legal in the current state.
    IllegalState,
    /// An unclassified internal failure.
    Internal,
}

impl ErrorCode {
    /// Anchor into `docs/errors.md`, so every surfaced error is documentable.
    #[must_use]
    pub fn doc_anchor(self) -> &'static str {
        match self {
            Self::CaptureUnavailable => "e_capture_unavailable",
            Self::CapturePermission => "e_capture_permission",
            Self::CaptureSourceGone => "e_capture_source_gone",
            Self::CaptureBackpressure => "e_capture_backpressure",
            Self::AudioNoLoopback => "e_audio_no_loopback",
            Self::AudioDeviceLost => "e_audio_device_lost",
            Self::CameraBusy => "e_camera_busy",
            Self::CameraNone => "e_camera_none",
            Self::CameraPermission => "e_camera_permission",
            Self::EncodeMissingBinary => "e_encode_missing_binary",
            Self::EncodeFailed => "e_encode_failed",
            Self::EncodeDied => "e_encode_died",
            Self::PublishPortsBusy => "e_publish_ports_busy",
            Self::PublishAssetsMissing => "e_publish_assets_missing",
            Self::PublishTimeout => "e_publish_timeout",
            Self::PublishPageError => "e_publish_page_error",
            Self::DiskFull => "e_disk_full",
            Self::IllegalState => "e_illegal_state",
            Self::Internal => "e_internal",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bgra_byte_len_matches_1080p() {
        let size = Size {
            width: 1920,
            height: 1080,
        };
        // 8.29 MB per frame is the number the buffer pool and the 249 MB/s figure derive from.
        assert_eq!(size.bgra_byte_len(), 1920 * 1080 * 4);
        assert_eq!(size.bgra_byte_len(), 8_294_400);
    }

    #[test]
    fn default_edit_is_passthrough() {
        // Drives FFmpeg's `-c copy` fast path; getting this wrong means a needless re-encode.
        assert!(EditSpec::default().is_passthrough());
    }

    #[test]
    fn trimming_is_not_passthrough() {
        let edit = EditSpec {
            start_ms: Some(1_000),
            ..EditSpec::default()
        };
        assert!(!edit.is_passthrough());
    }

    #[test]
    fn a_zero_start_trim_is_still_passthrough() {
        let edit = EditSpec {
            start_ms: Some(0),
            ..EditSpec::default()
        };
        assert!(edit.is_passthrough());
    }

    #[test]
    fn any_video_adjustment_defeats_passthrough() {
        let edit = EditSpec {
            video: VideoAdjust {
                brightness: 0.05,
                ..VideoAdjust::default()
            },
            ..EditSpec::default()
        };
        assert!(!edit.is_passthrough());
    }

    #[test]
    fn session_id_renders_as_fixed_width_hex() {
        assert_eq!(SessionId(0x1f).to_string(), "000000000000001f");
    }

    #[test]
    fn every_error_code_has_a_distinct_anchor() {
        let codes = [
            ErrorCode::CaptureUnavailable,
            ErrorCode::CapturePermission,
            ErrorCode::CaptureSourceGone,
            ErrorCode::CaptureBackpressure,
            ErrorCode::AudioNoLoopback,
            ErrorCode::AudioDeviceLost,
            ErrorCode::CameraBusy,
            ErrorCode::CameraNone,
            ErrorCode::CameraPermission,
            ErrorCode::EncodeMissingBinary,
            ErrorCode::EncodeFailed,
            ErrorCode::EncodeDied,
            ErrorCode::PublishPortsBusy,
            ErrorCode::PublishAssetsMissing,
            ErrorCode::PublishTimeout,
            ErrorCode::PublishPageError,
            ErrorCode::DiskFull,
            ErrorCode::IllegalState,
            ErrorCode::Internal,
        ];
        let mut anchors: Vec<&str> = codes.iter().map(|c| c.doc_anchor()).collect();
        anchors.sort_unstable();
        let before = anchors.len();
        anchors.dedup();
        assert_eq!(anchors.len(), before, "two ErrorCodes share a doc anchor");
    }
}
