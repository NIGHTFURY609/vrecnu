//! Webcam ownership.
//!
//! **The camera is opened exactly once, here, and its frames are fanned out.** That is the
//! entire reason this crate exists as a separate thing rather than living in the bubble.
//!
//! The tempting alternative was to let a transparent webview render `getUserMedia` for the
//! floating bubble while the engine opened the device again natively for the recording
//! overlay. Windows usually multiplexes that through the Camera Frame Server — but
//! DirectShow-era, OEM and virtual cameras (OBS Virtual Cam, vendor "beauty filter" drivers)
//! take exclusive control and fail the second open. That failure lands *after* the user has
//! pressed record, which makes it disqualifying rather than merely unfortunate.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod error;

pub use error::CameraError;

use vrecnu_types::{CameraInfo, Size};

/// One decoded camera frame, shared by every consumer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CameraFrame {
    /// Frame width.
    pub width: u32,
    /// Frame height.
    pub height: u32,
    /// Tightly packed BGRA8.
    pub bgra: Vec<u8>,
    /// QueryPerformanceCounter timestamp.
    pub qpc: i64,
}

/// Anything that wants camera frames: the bubble window, the encoder overlay pipe, a test.
pub trait CameraConsumer: Send {
    /// Called once per decoded frame, on the camera thread.
    fn on_camera_frame(&mut self, frame: &CameraFrame);
}

/// Opens the device and broadcasts to N consumers.
pub trait CameraSource: Send {
    /// Cameras the system reports.
    fn enumerate(&self) -> Result<Vec<CameraInfo>, CameraError>;
    /// Opens `id` and begins decoding, requesting the smallest format at least `min_size`.
    fn start(&mut self, id: &str, min_size: Size) -> Result<(), CameraError>;
    /// Adds a consumer. Safe to call while running.
    fn subscribe(&mut self, consumer: Box<dyn CameraConsumer>);
    /// Closes the device.
    fn stop(&mut self) -> Result<(), CameraError>;
}

// Re-exported from vrecnu-types so the camera pipeline and the bubble renderer provably
// use the identical crop.
pub use vrecnu_types::square_crop;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_landscape_frame_is_cropped_from_the_sides() {
        assert_eq!(square_crop(640, 480), (80, 0, 480));
    }

    #[test]
    fn a_portrait_frame_is_cropped_top_and_bottom() {
        assert_eq!(square_crop(480, 640), (0, 80, 480));
    }

    #[test]
    fn a_square_frame_is_left_alone() {
        assert_eq!(square_crop(512, 512), (0, 0, 512));
    }

    #[test]
    fn a_widescreen_sensor_keeps_the_full_height() {
        // 1920x1080 must yield a 1080 square, not a squashed 1920-wide circle.
        assert_eq!(square_crop(1920, 1080), (420, 0, 1080));
    }
}
