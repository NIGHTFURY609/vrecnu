//! Screen capture.
//!
//! Two things live here, and only one of them is obvious. The obvious one is a
//! [`CaptureBackend`] over Windows Graphics Capture. The other is the **pacer**, which is
//! where "0 dropped frames at 1080p30" (AGENTS.md §1) is actually won or lost.
//!
//! > Windows Graphics Capture delivers a frame only when the content *changes*. A static
//! > screen for four seconds delivers nothing. Fed straight into
//! > `ffmpeg -f rawvideo -framerate 30`, which assumes a constant frame rate, a five-minute
//! > recording renders as three minutes — and every frame counter reports success while the
//! > output is wrong.
//!
//! So the pacer ticks at the target rate and re-sends the previous frame when nothing new
//! arrived. Duplicates are counted separately from drops: duplication is expected and cheap
//! (the encoder emits a tiny P-frame), whereas a drop means the encoder fell behind and has
//! to be surfaced.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod pacer;
mod pool;
#[cfg(windows)]
mod wgc;

pub use error::CaptureError;
pub use pacer::{PaceDecision, Pacer};
pub use pool::{FramePool, PooledFrame};
#[cfg(windows)]
pub use wgc::{
    is_monitor_source, monitor_source_id, primary_monitor_source_id, window_source_id, WgcBackend,
};

use vrecnu_types::{CaptureSource, CaptureSourceId, Size};

/// Receives frames from the OS capture callback.
pub trait FrameSink: Send {
    /// Called on the capture thread. **Must return in under ~2 ms**: Windows Graphics Capture
    /// drops frames itself when the callback is slow, so anything beyond one memcpy belongs
    /// downstream.
    fn on_frame(&mut self, frame: FrameRef<'_>);
}

/// A borrowed view of one captured frame.
#[derive(Debug)]
pub struct FrameRef<'a> {
    /// Raw BGRA8 bytes, with `stride`-aligned rows.
    pub data: &'a [u8],
    /// **Row pitch, which is not `width * 4`.** Capture surfaces are alignment-padded, so
    /// copying row-by-row (or cropping) is mandatory; treating it as packed skews the image.
    pub stride: usize,
    /// Frame width in pixels.
    pub width: u32,
    /// Frame height in pixels.
    pub height: u32,
    /// QueryPerformanceCounter timestamp, used for pacing and A/V sync.
    pub qpc: i64,
}

/// Swappable capture implementation.
///
/// `windows-capture` is the intended backend: it exposes the mapped staging texture, so a
/// frame can be copied into a pooled buffer. `scap` hands back an owned `Vec<u8>` per frame,
/// which at 1080p30 is an 8 MB allocate-and-free thirty times a second. AGENTS.md §3 permits
/// either, and this trait keeps the choice reversible.
pub trait CaptureBackend: Send {
    /// Enumerates capturable monitors and windows.
    fn enumerate(&self) -> Result<Vec<CaptureSource>, CaptureError>;
    /// Begins delivering frames to `sink`.
    fn start(
        &mut self,
        source: &CaptureSourceId,
        sink: Box<dyn FrameSink>,
    ) -> Result<(), CaptureError>;
    /// Stops delivery and releases the capture session.
    fn stop(&mut self) -> Result<(), CaptureError>;
    /// Frame size the backend delivers; known only after `start`.
    fn frame_size(&self) -> Option<Size>;
}
