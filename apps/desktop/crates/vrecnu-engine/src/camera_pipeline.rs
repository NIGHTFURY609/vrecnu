//! Camera → bubble window, and camera → named pipe → FFmpeg overlay.
//!
//! Mirrors [`crate::audio_pipeline`] deliberately: a device callback that must not block,
//! delivered on its own thread, drained by a writer loop ticking on absolute deadlines anchored
//! to the recording's shared clock. Read that module first; only the differences are explained
//! here.
//!
//! **The device is opened exactly once, and both consumers are fed from that one open.** That
//! is the whole reason `vrecnu-camera` exists as its own crate — see its module doc comment for
//! the failure mode (virtual and OEM camera drivers take exclusive control and fail a second
//! open, *after* the user has pressed record). It follows that the bubble window cannot open
//! its own camera, so it is fed here rather than owning anything.
//!
//! **One composite, two alpha conventions.** The bubble window needs premultiplied BGRA and
//! FFmpeg's `bgra` input needs straight alpha. Compositing twice would double the per-frame
//! cost in the one window AGENTS.md §1 has no CPU headroom to spare, so the frame is composited
//! once in straight alpha and the premultiplied copy is derived from it — exact, and one
//! multiply per channel. See `vrecnu_bubble_win32::premultiply_into`.
//!
//! **The overlay's placement is fixed when FFmpeg spawns.** `overlay=x=..:y=..` is a
//! filter-graph argument, so dragging the bubble mid-recording moves the window but not the
//! burn-in. The position is read from the bubble once, at start, and reported by
//! [`CameraPipeline::overlay_position`].

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use parking_lot::Mutex;
use vrecnu_types::{BubbleSpec, CameraInfo, Size};

use crate::EngineError;

/// How long [`CameraPipeline::stop`] waits for the writer thread before abandoning it.
///
/// Same trade-off, same reasoning, and for the same two cases as `audio_pipeline::STOP_GRACE`
/// — read that constant's doc comment. Shorter here only because the camera writer's per-tick
/// payload is one bubble surface (~309 KB at the default diameter) rather than a stream that
/// FFmpeg must keep in sync with audio, so a stall costs a frame rather than a desync.
const STOP_GRACE: Duration = Duration::from_secs(5);

static PIPE_COUNTER: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);

fn unique_pipe_name() -> String {
    let n = PIPE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("vrecnu-camera-{}-{n}", std::process::id())
}

/// What to open, and whether to show the floating window.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CameraSelection {
    /// Device id from [`list_cameras`], or `Some(String::new())` for the system default.
    /// `None` records without a camera at all.
    pub device: Option<String>,
    /// Shape and placement of the floating bubble. `None` burns the camera into the recording
    /// without showing a window — which is what a headless `vrecnu-cli record` wants.
    pub bubble: Option<BubbleSpec>,
    /// Composite the camera into the recording through FFmpeg's `overlay` filter.
    ///
    /// **Must be false when a bubble window is shown over a monitor being captured.** DWM
    /// composites the floating window onto the desktop, so Windows Graphics Capture already
    /// records it; burning it in as well draws the bubble twice. That is not a subtle artefact
    /// — it is two visibly offset circles, because the captured copy lands wherever the window
    /// actually is while the overlay lands at the coordinates FFmpeg was given, and nothing
    /// keeps those two in step once the user drags the window.
    ///
    /// Still needed without a window (nothing on screen to capture) and for a *window* capture
    /// (the bubble is a separate window, so it is not in that capture).
    pub burn_in: bool,
}

impl CameraSelection {
    /// True when there is nothing for this module to do.
    const fn is_none(&self) -> bool {
        self.device.is_none()
    }
}

/// A live camera pipeline: the device, the bubble window, and the pipe FFmpeg overlays from.
///
/// Constructed by [`start`] before FFmpeg is spawned (so the pipe exists for FFmpeg's `-i` to
/// open) and told the recording's clock via [`begin`]. `Drop` stops everything, so letting this
/// go out of scope is the intended shutdown — see [`stop`].
///
/// [`start`]: start
/// [`begin`]: CameraPipeline::begin
/// [`stop`]: CameraPipeline::stop
pub struct CameraPipeline {
    /// `None` when the camera feeds only the bubble window — see `CameraSelection::burn_in`.
    pipe_name: Option<String>,
    overlay_size: Size,
    overlay_position: Option<(i32, i32)>,
    stop_flag: Arc<AtomicBool>,
    start_tx: Option<mpsc::Sender<Instant>>,
    writer: Option<JoinHandle<()>>,
    device: Option<Box<dyn imp::CameraHandle>>,
}

impl CameraPipeline {
    /// The full `\\.\pipe\...` path — pass this as `LiveEncodeParams::camera_pipe`.
    ///
    /// `None` when the camera is not being burned in, in which case FFmpeg gets no overlay
    /// input at all and `overlay_size`/`overlay_position` are meaningless.
    #[must_use]
    pub fn pipe_path(&self) -> Option<String> {
        self.pipe_name.as_deref().map(vrecnu_namedpipe::pipe_path)
    }

    /// Overlay frame size — pass this as `LiveEncodeParams::camera_size`. Meaningless unless
    /// [`Self::pipe_path`] is `Some`.
    ///
    /// This is the *window* size, shadow margin included, not the circle: the surface written to
    /// the pipe is the whole layered-window surface, and its transparent margin is what lets the
    /// drop shadow blend into the recording instead of ending at a hard square edge.
    #[must_use]
    pub const fn overlay_size(&self) -> Size {
        self.overlay_size
    }

    /// Where the overlay should be burned in — pass this as `LiveEncodeParams::camera_position`.
    ///
    /// `None` when no bubble window is shown, which leaves FFmpeg's corner default in place.
    #[must_use]
    pub const fn overlay_position(&self) -> Option<(i32, i32)> {
        self.overlay_position
    }

    /// Unblocks the writer thread's tick loop, anchored to `started`.
    ///
    /// Call exactly once, with the same instant `audio_pipeline::AudioPipeline::begin` is given
    /// — the first video frame's. Three inputs into one muxer means three clocks, and the only
    /// way they stay in step is that none of them measures its own zero.
    pub fn begin(&mut self, started: Instant) {
        if let Some(tx) = self.start_tx.take() {
            // An error means the writer thread already exited, most likely because FFmpeg never
            // opened the camera pipe. `stop()` reports that through its own bounded wait rather
            // than this call surfacing the same condition twice.
            let _ = tx.send(started);
        }
    }

    /// Stops the camera, then the writer thread, then drops the pipe — giving FFmpeg's overlay
    /// input a clean EOF, the same "close rather than kill" finalization the video stdin and the
    /// audio pipe both get.
    ///
    /// Idempotent, and takes `&mut self` rather than consuming, for exactly the reason
    /// `AudioPipeline::stop` does: `run_until_stopped` has several `?`-driven exits and relies
    /// on scope-based drop to cover all of them uniformly.
    pub fn stop(&mut self) {
        // Device first: no more frames should reach the consumers once we are shutting down,
        // and closing the device is also what turns the webcam's activity light off — which a
        // user reasonably reads as "it has stopped watching me".
        if let Some(device) = self.device.as_mut() {
            device.stop();
        }
        self.device = None;

        self.stop_flag.store(true, Ordering::Relaxed);
        let Some(handle) = self.writer.take() else {
            return;
        };

        let (done_tx, done_rx) = mpsc::channel::<()>();
        // Joined on a throwaway thread so a writer stuck in `ConnectNamedPipe` — no client ever
        // arrived — costs this call `STOP_GRACE` rather than the rest of the process's life.
        std::thread::spawn(move || {
            let _ = handle.join();
            let _ = done_tx.send(());
        });

        if done_rx.recv_timeout(STOP_GRACE).is_err() {
            tracing::warn!(
                "camera pipeline writer thread did not stop within {STOP_GRACE:?}; abandoning it"
            );
        }
    }
}

impl Drop for CameraPipeline {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Starts the camera, the bubble window and the named pipe. Returns `Ok(None)` when no camera
/// was asked for.
///
/// Blocks until the device is confirmed open, so a busy or permission-denied camera surfaces
/// here — before FFmpeg is spawned and before the user believes recording has started. That is
/// the same discipline `record()` applies to the H.264 encoder probe and `audio_pipeline::start`
/// applies to WASAPI, and on this path it is the point of the whole crate: the failure being
/// avoided is a second device open failing *after* the record button.
///
/// # Errors
/// [`EngineError::Pipe`] if the named pipe cannot be created, [`EngineError::Camera`] if the
/// device cannot be opened.
pub fn start(selection: &CameraSelection, fps: u32) -> Result<Option<CameraPipeline>, EngineError> {
    if selection.is_none() {
        return Ok(None);
    }
    imp::start(selection, fps)
}

/// Cameras the system reports, without opening any of them.
///
/// # Errors
/// [`EngineError::Camera`] if the platform has no camera backend or the backend will not answer.
pub fn list_cameras() -> Result<Vec<CameraInfo>, EngineError> {
    imp::list_cameras()
}

/// Whether at least one camera exists, for a capability report like `doctor`.
#[must_use]
pub fn probe() -> bool {
    imp::probe()
}

#[cfg(windows)]
mod imp {
    use super::{unique_pipe_name, CameraPipeline, CameraSelection, EngineError, Mutex, Ordering};
    use std::sync::atomic::AtomicBool;
    use std::sync::{mpsc, Arc};
    use std::time::{Duration, Instant};

    use vrecnu_bubble_win32::{
        premultiply_into, to_square_into, AlphaMode, BubbleLayout, BubbleMask, LayeredBubble,
        SquareFrame,
    };
    use vrecnu_camera::{CameraConsumer, CameraFrame, CameraSource, NokhwaCameraSource};
    use vrecnu_namedpipe::NamedPipeServer;
    use vrecnu_types::{CameraInfo, Size};

    /// Erases the concrete source type, so [`CameraPipeline`] holds a device without naming a
    /// backend — the same shape `audio_pipeline::AudioSourceHandle` uses.
    pub(crate) trait CameraHandle: Send {
        fn stop(&mut self);
    }

    struct Started<S: CameraSource>(S);
    impl<S: CameraSource> CameraHandle for Started<S> {
        fn stop(&mut self) {
            let _ = self.0.stop();
        }
    }

    pub(super) fn list_cameras() -> Result<Vec<CameraInfo>, EngineError> {
        Ok(vrecnu_camera::enumerate_cameras()?)
    }

    pub(super) fn probe() -> bool {
        vrecnu_camera::camera_available()
    }

    /// The one consumer. Composites each camera frame once and fans the result both ways.
    ///
    /// Runs on the camera thread, so it must be cheap and must not do I/O: it stores the
    /// overlay surface for the writer thread to pick up on its own tick, and hands the bubble
    /// window a copy through `present`, which itself only stages bytes and posts a message.
    struct Compositor {
        mask: BubbleMask,
        square: SquareFrame,
        /// Straight-alpha surface, for FFmpeg. Also the source the premultiplied copy is
        /// derived from.
        straight: Vec<u8>,
        /// Premultiplied copy, for `UpdateLayeredWindow`. Absent when no window is shown.
        premultiplied: Option<Vec<u8>>,
        latest: Arc<Mutex<Vec<u8>>>,
        bubble: Option<Arc<LayeredBubble>>,
    }

    impl CameraConsumer for Compositor {
        fn on_camera_frame(&mut self, frame: &CameraFrame) {
            to_square_into(&frame.bgra, frame.width, frame.height, &mut self.square);
            self.mask
                .composite_into(&self.square, AlphaMode::Straight, &mut self.straight);

            // A short memcpy under the lock, rather than compositing under it: the writer
            // thread must never wait on a full rasterisation to take its tick.
            self.latest.lock().copy_from_slice(&self.straight);

            if let (Some(bubble), Some(premultiplied)) =
                (self.bubble.as_ref(), self.premultiplied.as_mut())
            {
                premultiply_into(&self.straight, premultiplied);
                // A closed window is not an error worth failing a recording over — the burn-in
                // carries on regardless, which is the half the user is actually keeping.
                let _ = bubble.present(premultiplied);
            }
        }
    }

    pub(super) fn start(
        selection: &CameraSelection,
        fps: u32,
    ) -> Result<Option<CameraPipeline>, EngineError> {
        let spec = selection.bubble.unwrap_or_default();
        let layout = BubbleLayout::from_spec(spec);
        let mask = BubbleMask::new(&layout);
        let surface_len = mask.surface_len();
        let camera_side = mask.camera_side();

        // Starts fully transparent, which is exactly right: FFmpeg's `overlay` composites every
        // tick from the moment it opens the pipe, and before the first camera frame there is
        // genuinely nothing to draw. A frame of zeros is a no-op overlay; skipping the write
        // instead would starve a fixed-rate rawvideo input.
        let latest = Arc::new(Mutex::new(vec![0u8; surface_len]));

        // The window is opened before the device, so a camera that takes a second to warm up
        // does not leave the user staring at nothing wondering whether it worked.
        let bubble = match selection.bubble {
            Some(spec) => Some(Arc::new(LayeredBubble::spawn(spec)?)),
            None => None,
        };

        let compositor = Compositor {
            mask,
            square: SquareFrame::solid(camera_side, [0, 0, 0, 255]),
            straight: vec![0u8; surface_len],
            premultiplied: bubble.as_ref().map(|_| vec![0u8; surface_len]),
            latest: Arc::clone(&latest),
            bubble: bubble.clone(),
        };

        let mut source = NokhwaCameraSource::new();
        // Subscribed *before* `start`, so no frame can be delivered and dropped in the gap
        // between the device opening and the consumer being attached.
        source.subscribe(Box::new(compositor));
        let device_id = selection.device.clone().unwrap_or_default();
        source.start(
            &device_id,
            Size {
                width: camera_side,
                height: camera_side,
            },
        )?;

        let stop_flag = Arc::new(AtomicBool::new(false));
        let (start_tx, start_rx) = mpsc::channel::<Instant>();
        let tick = Duration::from_nanos(1_000_000_000 / u64::from(fps.max(1)));

        // No burn-in means no pipe, no writer thread and no overlay input on FFmpeg's command
        // line at all -- the camera exists purely to feed the bubble window, which the screen
        // capture already picks up. See `CameraSelection::burn_in`.
        let (pipe_name, writer) = if selection.burn_in {
            let pipe_name = unique_pipe_name();
            // Two whole overlay frames, not the default 64 KB. One frame is ~309 KB at the
            // default diameter, so a 64 KB buffer means every write blocks part-way through
            // until FFmpeg reads -- and during `h264_mf`'s warm-up backlog that is seconds.
            let server = NamedPipeServer::create_with_buffer(
                &pipe_name,
                u32::try_from(surface_len.saturating_mul(2)).unwrap_or(u32::MAX),
            )?;
            let stop_for_writer = Arc::clone(&stop_flag);
            let writer = std::thread::spawn(move || {
                writer_loop(&server, &latest, &stop_for_writer, &start_rx, tick);
            });
            (Some(pipe_name), Some(writer))
        } else {
            (None, None)
        };

        Ok(Some(CameraPipeline {
            pipe_name,
            overlay_size: Size {
                width: layout.window.width,
                height: layout.window.height,
            },
            // Read once, here. `overlay` coordinates are fixed when FFmpeg spawns, so a later
            // drag moves the window and not the burn-in — see the module doc comment.
            overlay_position: bubble
                .as_ref()
                .map(|bubble| (bubble.geometry().x, bubble.geometry().y)),
            stop_flag,
            start_tx: Some(start_tx),
            writer,
            device: Some(Box::new(Started(source))),
        }))
    }

    /// Writes one overlay frame per tick, on absolute deadlines anchored to the shared clock.
    ///
    /// A fixed-rate `rawvideo` input has to receive a frame per interval whether or not the
    /// camera produced one — the same duplicate-rather-than-skip rule the video pacer follows.
    /// Sending only on new camera frames would make FFmpeg's overlay input run at whatever rate
    /// the webcam happened to manage, and the recording would drift against it.
    fn writer_loop(
        server: &NamedPipeServer,
        latest: &Arc<Mutex<Vec<u8>>>,
        stop: &AtomicBool,
        start_rx: &mpsc::Receiver<Instant>,
        tick: Duration,
    ) {
        // No client yet — FFmpeg is spawned by `record()` after this thread starts, so blocking
        // here for a real stretch of wall-clock time is the design, not a stall.
        if server.wait_for_client().is_err() {
            return;
        }

        // Returns without writing anything if the recording aborted before a single video frame
        // arrived: there was never a clock to tick against.
        let Ok(started) = start_rx.recv() else {
            return;
        };

        // Copied out from under the lock each tick so the pipe write — which can block for as
        // long as FFmpeg takes to read — never holds up the camera thread's next composite.
        let mut frame = latest.lock().clone();
        let mut next_tick = started;
        while !stop.load(Ordering::Relaxed) {
            next_tick += tick;
            crate::pipeline::sleep_until(next_tick);
            if stop.load(Ordering::Relaxed) {
                break;
            }

            {
                let current = latest.lock();
                if current.len() == frame.len() {
                    frame.copy_from_slice(&current);
                } else {
                    frame = current.clone();
                }
            }

            if server.write_all(&frame).is_err() {
                // FFmpeg's overlay input died. The video writer notices the same dead process
                // independently from its own stdin write; nothing useful left to do here.
                break;
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{CameraPipeline, CameraSelection, EngineError};
    use vrecnu_types::CameraInfo;

    pub(crate) trait CameraHandle: Send {
        fn stop(&mut self);
    }

    pub(super) fn start(
        _selection: &CameraSelection,
        _fps: u32,
    ) -> Result<Option<CameraPipeline>, EngineError> {
        Err(EngineError::Camera(vrecnu_camera::CameraError::Backend(
            "camera capture is only implemented on Windows".into(),
        )))
    }

    pub(super) fn list_cameras() -> Result<Vec<CameraInfo>, EngineError> {
        Ok(Vec::new())
    }

    pub(super) fn probe() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_camera_starts_nothing() {
        let result = start(&CameraSelection::default(), 30);
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn a_selection_without_a_device_is_none_even_when_a_bubble_is_asked_for() {
        // A bubble with no camera in it would be an empty circle floating over the desktop.
        let selection = CameraSelection {
            device: None,
            bubble: Some(BubbleSpec::default()),
            burn_in: true,
        };
        assert!(selection.is_none());
    }

    #[test]
    fn the_default_device_is_expressed_as_an_empty_id_not_a_missing_one() {
        // `None` means "no camera"; `Some("")` means "whichever camera the system defaults to".
        // Collapsing the two would make it impossible to record without a webcam.
        let selection = CameraSelection {
            device: Some(String::new()),
            bubble: None,
            burn_in: true,
        };
        assert!(!selection.is_none());
    }

    #[test]
    fn pipe_names_are_unique_across_calls() {
        assert_ne!(unique_pipe_name(), unique_pipe_name());
    }

    #[test]
    fn camera_and_audio_pipe_names_cannot_collide() {
        // Both are `\\.\pipe\vrecnu-<kind>-<pid>-<n>` and both counters start at zero, so the
        // kind is the only thing keeping a camera pipe from claiming the audio pipe's name --
        // and `NamedPipeServer::create` fails outright on a collision, which would surface as a
        // recording that refuses to start for no visible reason.
        assert!(unique_pipe_name().starts_with("vrecnu-camera-"));
    }
}
