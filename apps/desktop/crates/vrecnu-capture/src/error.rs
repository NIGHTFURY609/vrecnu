use vrecnu_types::{CaptureSourceId, PixelFormat};

/// Failures specific to screen capture.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CaptureError {
    /// Windows Graphics Capture requires Windows 10 1903 or newer.
    #[error(
        "screen capture is unavailable on this version of Windows (requires 10 1903 or newer)"
    )]
    Unavailable,
    /// The chosen window or monitor no longer exists.
    #[error("capture source {0:?} no longer exists")]
    SourceGone(CaptureSourceId),
    /// The user declined or revoked capture permission.
    #[error("screen capture permission was denied")]
    PermissionDenied,
    /// The encoder fell behind far enough that frames had to be discarded.
    #[error("the encoder is {behind_ms} ms behind and frames are being dropped")]
    Backpressure {
        /// How far behind the encoder is.
        behind_ms: u64,
    },
    /// Capture started but the OS never delivered a first frame.
    ///
    /// Distinct from a stalled capture mid-session: Windows Graphics Capture is change-driven,
    /// so *no frames at all* means the session never really started (a source that is minimised,
    /// occluded on a headless session, or blocked by policy), whereas silence later on is
    /// normal and the pacer's job.
    #[error("no frames arrived from the capture source within {after_ms} ms")]
    NoFrames {
        /// How long the pipeline waited.
        after_ms: u64,
    },
    /// The source changed resolution mid-recording.
    ///
    /// The encoder's `-video_size` is fixed at spawn, so this cannot be absorbed silently.
    #[error("capture source changed from {from_w}x{from_h} to {to_w}x{to_h} mid-recording")]
    SizeChanged {
        /// Width capture started at.
        from_w: u32,
        /// Height capture started at.
        from_h: u32,
        /// Width it changed to.
        to_w: u32,
        /// Height it changed to.
        to_h: u32,
    },
    /// The backend delivered something other than BGRA8.
    #[error("unexpected frame format {got:?}, expected BGRA8")]
    UnexpectedFormat {
        /// What arrived instead.
        got: PixelFormat,
    },
    /// Anything the backend raised that has no better classification.
    #[error("capture backend failed: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}
