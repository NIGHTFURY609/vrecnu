//! Windows Graphics Capture backend.
//!
//! `windows-capture` is used rather than `scap` for one concrete reason: it exposes the
//! mapped staging texture, so a frame can be copied into a buffer we already own. `scap`
//! hands back an owned `Vec<u8>` per frame, which at 1080p30 is an 8 MB allocate-and-free
//! thirty times a second — a quarter of a gigabyte per second of pure allocator churn.
//! AGENTS.md §3 permits either; [`crate::CaptureBackend`] keeps the choice reversible.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

use vrecnu_types::{CaptureSource, CaptureSourceId, CaptureSourceKind, Size};
use windows_capture::capture::{CaptureControl, Context, GraphicsCaptureApiHandler};
use windows_capture::frame::Frame;
use windows_capture::graphics_capture_api::InternalCaptureControl;
use windows_capture::monitor::Monitor;
use windows_capture::settings::{
    ColorFormat, CursorCaptureSettings, DirtyRegionSettings, DrawBorderSettings,
    MinimumUpdateIntervalSettings, SecondaryWindowSettings, Settings,
};
use windows_capture::window::Window;

use crate::{CaptureBackend, CaptureError, FrameRef, FrameSink};

/// Prefix marking a source id as a monitor index.
const MONITOR_PREFIX: &str = "monitor:";
/// Prefix marking a source id as a window handle.
const WINDOW_PREFIX: &str = "window:";

/// Builds the id string the UI round-trips back to [`WgcBackend::start`].
///
/// **`index` is a Windows display *device* number**, the `5` in `\\.\DISPLAY5`, which is what
/// `Monitor::index()` returns. It is one-based, stable across reboots, and — importantly — it is
/// *not* a position in [`Monitor::enumerate`]'s list. Resolve one with [`monitor_by_index`],
/// never with `Monitor::from_index`; see that function's doc comment for what goes wrong.
#[must_use]
pub fn monitor_source_id(index: usize) -> CaptureSourceId {
    CaptureSourceId(format!("{MONITOR_PREFIX}{index}"))
}

/// Finds the monitor whose display device number is `index`.
///
/// **Not `Monitor::from_index`.** In `windows-capture` 2.0 those two are different namespaces
/// that both happen to be one-based, which is what made the confusion survive review:
///
/// - `Monitor::index()` parses the number out of the device name — `\\.\DISPLAY5` gives `5`.
/// - `Monitor::from_index(n)` returns `enumerate()[n - 1]`, a *position* in the list.
///
/// They agree only when a display's device number equals its enumeration position. On a laptop
/// whose single panel is `\\.\DISPLAY5`, `index()` returns 5 and `from_index(5)` returns
/// `NotFound` — so `sources` listed `monitor:5` and starting it reported that the display had
/// been disconnected. Every recording on such a machine failed, and the error blamed the user's
/// monitor. Matching on the device number instead is also order-independent, which is what a
/// multi-monitor setup needs: unplugging a display renumbers positions but not device names.
#[must_use]
pub fn monitor_by_index(index: usize) -> Option<Monitor> {
    Monitor::enumerate()
        .ok()?
        .into_iter()
        .find(|monitor| monitor.index().ok() == Some(index))
}

/// The id for the primary display, for callers that did not ask for a specific source.
///
/// # Errors
/// [`CaptureError::Backend`] if the primary monitor cannot be resolved.
pub fn primary_monitor_source_id() -> Result<CaptureSourceId, CaptureError> {
    let monitor = Monitor::primary().map_err(|err| CaptureError::Backend(Box::new(err)))?;
    let index = monitor
        .index()
        .map_err(|err| CaptureError::Backend(Box::new(err)))?;
    Ok(monitor_source_id(index))
}

/// Whether `id` names a monitor rather than a window.
///
/// Lives here, next to the prefix constants, so the answer cannot drift from the format
/// `monitor_source_id` produces. The engine needs it because a floating bubble is composited
/// onto the desktop by DWM and therefore appears in a *monitor* capture on its own, but is not
/// part of any single *window* capture.
#[must_use]
pub fn is_monitor_source(id: &CaptureSourceId) -> bool {
    id.0.starts_with(MONITOR_PREFIX)
}

/// Builds a window source id from its raw `HWND` value.
#[must_use]
pub fn window_source_id(hwnd: isize) -> CaptureSourceId {
    CaptureSourceId(format!("{WINDOW_PREFIX}{hwnd}"))
}

/// Where Windows parks a minimized window's rectangle.
const MINIMIZED_ORIGIN: i32 = -32_000;

/// A window's size, from its screen rectangle (`GetWindowRect`).
///
/// This is the outer frame. It includes the invisible resize borders (a few pixels a side on
/// Windows 10/11) that WGC does not capture, so it is a label for the picker, not the encode
/// size: the pipeline takes that from the first frame (HANDOFF, M2 wiring note 2). A minimized
/// window reports `0x0` rather than the taskbar stub Windows parks at `-32000`: its real size
/// is unknown until it is restored.
fn window_size(left: i32, top: i32, right: i32, bottom: i32) -> Size {
    if left <= MINIMIZED_ORIGIN && top <= MINIMIZED_ORIGIN {
        return Size {
            width: 0,
            height: 0,
        };
    }
    let span = |from: i32, to: i32| u32::try_from(to.saturating_sub(from)).unwrap_or(0);
    Size {
        width: span(left, right),
        height: span(top, bottom),
    }
}

/// Wraps the sink so the handler can own it while the caller keeps a stop handle.
struct SinkHandler {
    sink: Box<dyn FrameSink>,
    /// Written by the handler, read by the owner to detect a stalled capture.
    last_qpc: Arc<AtomicI64>,
}

impl GraphicsCaptureApiHandler for SinkHandler {
    type Flags = (Box<dyn FrameSink>, Arc<AtomicI64>);
    type Error = CaptureError;

    fn new(ctx: Context<Self::Flags>) -> Result<Self, Self::Error> {
        let (sink, last_qpc) = ctx.flags;
        Ok(Self { sink, last_qpc })
    }

    fn on_frame_arrived(
        &mut self,
        frame: &mut Frame,
        _control: InternalCaptureControl,
    ) -> Result<(), Self::Error> {
        // This runs on the capture thread. Everything here must stay under ~2 ms: Windows
        // Graphics Capture drops frames itself when the callback is slow, so the sink does a
        // single memcpy and returns.
        let timestamp = frame
            .timestamp()
            .map(|span| span.Duration)
            .unwrap_or_default();

        let mut buffer = frame
            .buffer()
            .map_err(|err| CaptureError::Backend(Box::new(err)))?;

        let width = buffer.width();
        let height = buffer.height();
        // Row pitch, NOT width * 4. Staging textures are alignment-padded; treating the
        // buffer as tightly packed produces the classic skewed image.
        let stride = buffer.row_pitch() as usize;
        let data = buffer.as_raw_buffer();

        self.last_qpc.store(timestamp, Ordering::Relaxed);
        self.sink.on_frame(FrameRef {
            data,
            stride,
            width,
            height,
            qpc: timestamp,
        });
        Ok(())
    }

    fn on_closed(&mut self) -> Result<(), Self::Error> {
        tracing::info!("capture source closed");
        Ok(())
    }
}

/// Screen capture over Windows Graphics Capture.
#[derive(Default)]
pub struct WgcBackend {
    control: Option<CaptureControl<SinkHandler, CaptureError>>,
    frame_size: Option<Size>,
    last_qpc: Arc<AtomicI64>,
}

impl std::fmt::Debug for WgcBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WgcBackend")
            .field("running", &self.control.is_some())
            .field("frame_size", &self.frame_size)
            .finish()
    }
}

impl WgcBackend {
    /// A backend that is not yet capturing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// QPC timestamp of the most recent frame, for stall detection.
    #[must_use]
    pub fn last_frame_qpc(&self) -> i64 {
        self.last_qpc.load(Ordering::Relaxed)
    }
}

impl CaptureBackend for WgcBackend {
    fn enumerate(&self) -> Result<Vec<CaptureSource>, CaptureError> {
        let mut sources = Vec::new();

        let monitors = Monitor::enumerate().map_err(|err| CaptureError::Backend(Box::new(err)))?;
        let primary = Monitor::primary().ok();
        let primary_index = primary.and_then(|monitor| monitor.index().ok());

        for monitor in monitors {
            let Ok(index) = monitor.index() else { continue };
            let name = monitor
                .name()
                .unwrap_or_else(|_| format!("Display {index}"));
            let width = monitor.width().unwrap_or(0);
            let height = monitor.height().unwrap_or(0);
            sources.push(CaptureSource {
                id: monitor_source_id(index),
                title: name,
                kind: CaptureSourceKind::Monitor,
                size: Size { width, height },
                is_primary: Some(index) == primary_index,
            });
        }

        // Windows the user could plausibly pick. `Window::enumerate` already filters out
        // invisible and tool windows, so no extra heuristics are needed here.
        if let Ok(windows) = Window::enumerate() {
            for window in windows {
                let Ok(title) = window.title() else { continue };
                if title.trim().is_empty() {
                    continue;
                }
                sources.push(CaptureSource {
                    id: window_source_id(window.as_raw_hwnd() as isize),
                    title,
                    kind: CaptureSourceKind::Window,
                    size: window.rect().map_or(
                        Size {
                            width: 0,
                            height: 0,
                        },
                        |rect| window_size(rect.left, rect.top, rect.right, rect.bottom),
                    ),
                    is_primary: false,
                });
            }
        }

        Ok(sources)
    }

    fn start(
        &mut self,
        source: &CaptureSourceId,
        sink: Box<dyn FrameSink>,
    ) -> Result<(), CaptureError> {
        if self.control.is_some() {
            return Err(CaptureError::Backend("capture is already running".into()));
        }

        let flags = (sink, Arc::clone(&self.last_qpc));
        let raw = source.0.as_str();

        // Settings::new is inlined per branch rather than built by a helper closure: the
        // capture item is a different concrete type for a monitor and a window, and a closure
        // would be monomorphised to whichever one called it first.
        if let Some(index) = raw.strip_prefix(MONITOR_PREFIX) {
            // A display *device* number, not a position in the enumeration -- see
            // `monitor_by_index`, which is the only correct way to resolve one.
            let index: usize = index
                .parse()
                .map_err(|_| CaptureError::SourceGone(source.clone()))?;
            let monitor =
                monitor_by_index(index).ok_or_else(|| CaptureError::SourceGone(source.clone()))?;
            self.frame_size = Some(Size {
                width: monitor.width().unwrap_or(0),
                height: monitor.height().unwrap_or(0),
            });
            let settings = Settings::new(
                monitor,
                CursorCaptureSettings::WithCursor,
                DrawBorderSettings::WithoutBorder,
                SecondaryWindowSettings::Default,
                MinimumUpdateIntervalSettings::Default,
                DirtyRegionSettings::Default,
                // BGRA8 is what the pipeline and FFmpeg's `-pixel_format bgra` expect.
                // Asking for RGBA here would silently swap red and blue.
                ColorFormat::Bgra8,
                flags,
            );
            self.control = Some(
                SinkHandler::start_free_threaded(settings)
                    .map_err(|err| CaptureError::Backend(Box::new(err)))?,
            );
            Ok(())
        } else if let Some(handle) = raw.strip_prefix(WINDOW_PREFIX) {
            let handle: isize = handle
                .parse()
                .map_err(|_| CaptureError::SourceGone(source.clone()))?;
            let window = Window::from_raw_hwnd(handle as *mut std::ffi::c_void);
            let settings = Settings::new(
                window,
                CursorCaptureSettings::WithCursor,
                DrawBorderSettings::WithoutBorder,
                SecondaryWindowSettings::Default,
                MinimumUpdateIntervalSettings::Default,
                DirtyRegionSettings::Default,
                ColorFormat::Bgra8,
                flags,
            );
            self.control = Some(
                SinkHandler::start_free_threaded(settings)
                    .map_err(|err| CaptureError::Backend(Box::new(err)))?,
            );
            Ok(())
        } else {
            Err(CaptureError::SourceGone(source.clone()))
        }
    }

    fn stop(&mut self) -> Result<(), CaptureError> {
        if let Some(control) = self.control.take() {
            control
                .stop()
                .map_err(|err| CaptureError::Backend(Box::new(err)))?;
        }
        Ok(())
    }

    fn frame_size(&self) -> Option<Size> {
        self.frame_size
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_ids_round_trip() {
        let id = monitor_source_id(2);
        assert_eq!(id.0, "monitor:2");
        assert_eq!(id.0.strip_prefix(MONITOR_PREFIX), Some("2"));
    }

    #[test]
    fn window_ids_round_trip() {
        let id = window_source_id(123_456);
        assert_eq!(id.0, "window:123456");
    }

    #[test]
    fn monitor_and_window_ids_cannot_be_confused() {
        // They index different namespaces; a collision would capture the wrong thing.
        assert_ne!(monitor_source_id(5).0, window_source_id(5).0);
    }

    #[test]
    fn an_unrecognised_id_reports_the_source_as_gone() {
        let mut backend = WgcBackend::new();
        struct Discard;
        impl FrameSink for Discard {
            fn on_frame(&mut self, _frame: FrameRef<'_>) {}
        }
        let err = backend
            .start(&CaptureSourceId("nonsense".into()), Box::new(Discard))
            .unwrap_err();
        assert!(matches!(err, CaptureError::SourceGone(_)));
    }

    #[test]
    fn a_window_size_comes_from_its_rectangle() {
        assert_eq!(
            window_size(100, 50, 1380, 770),
            Size {
                width: 1280,
                height: 720
            }
        );
    }

    #[test]
    fn a_window_partly_off_screen_keeps_its_full_size() {
        assert_eq!(
            window_size(-200, -10, 1080, 710),
            Size {
                width: 1280,
                height: 720
            }
        );
    }

    #[test]
    fn a_minimized_window_reports_no_size_rather_than_its_taskbar_stub() {
        // Windows parks a minimized window at (-32000, -32000) with a ~160x28 rectangle. That
        // is not the size anyone would record.
        assert_eq!(
            window_size(-32_000, -32_000, -31_840, -31_972),
            Size {
                width: 0,
                height: 0
            }
        );
    }

    #[test]
    fn an_inverted_rectangle_reports_zero_rather_than_wrapping() {
        assert_eq!(
            window_size(10, 10, 5, 5),
            Size {
                width: 0,
                height: 0
            }
        );
    }

    #[test]
    fn stopping_a_backend_that_never_started_is_not_an_error() {
        assert!(WgcBackend::new().stop().is_ok());
    }
}
