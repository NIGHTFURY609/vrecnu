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
