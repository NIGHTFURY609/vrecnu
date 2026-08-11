/// Failures specific to camera capture.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CameraError {
    /// No camera is attached.
    #[error("no camera found")]
    NoCamera,
    /// Another application holds the device exclusively.
    ///
    /// The case that ruled out opening the camera twice: virtual and OEM drivers report this
    /// on a second open, and it would surface only after the user pressed record.
    #[error("camera {name} is in use by another application")]
    DeviceBusy {
        /// Device name.
        name: String,
    },
    /// The device offers no format we can decode at the requested size.
    #[error("camera {name} offers no usable format at {width}x{height} or larger")]
    FormatNegotiationFailed {
        /// Device name.
        name: String,
        /// Requested width.
        width: u32,
        /// Requested height.
        height: u32,
    },
    /// Camera access was denied by the OS privacy settings.
    #[error("camera access was denied")]
    PermissionDenied,
    /// A frame arrived but could not be decoded.
    #[error("could not decode a camera frame: {0}")]
    FrameDecode(String),
    /// Anything the backend raised that has no better classification.
    #[error("camera backend failed: {0}")]
    Backend(#[source] Box<dyn std::error::Error + Send + Sync>),
}
