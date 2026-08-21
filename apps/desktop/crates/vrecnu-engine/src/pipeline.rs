//! Capture → pacer → FFmpeg.
//!
//! Every piece this joins already existed and was unit-tested in isolation; this module is the
//! wiring, and the wiring is where the interesting failures live.
//!
//! Three of them are worth stating outright, because each produces a *plausible-looking* file
//! rather than an error:
//!
//! 1. **The clock starts at the first frame, not at `start()`.** Windows Graphics Capture takes
//!    tens of milliseconds to deliver anything. Starting the pacer before then spends the first
//!    few ticks with nothing to send, and a "10 second" recording comes out 299 frames long.
//! 2. **The frame size comes from the first frame, not from the monitor.** A display reports
//!    1920x1080 while the capture surface is 1920x1088 often enough to matter, and FFmpeg's
//!    `-video_size` is fixed at spawn. Reading it from the OS instead of from the pixels shifts
//!    every row by a few bytes and produces the classic diagonal smear.
//! 3. **Deadlines are absolute, never `now + interval`.** `thread::sleep` overshoots; adding the
//!    overshoot back in each iteration turns a 10-second recording into 10 seconds of frames
//!    sampled over 14 seconds of real time — correct duration, motion 40% too fast.
//!
//! Drops are counted only when a frame is *lost* (the pool is exhausted), never when the pacer
//! legitimately discards an extra frame from a 60 Hz screen being recorded at 30 fps. Conflating
//! the two makes the "`frames_dropped == 0`" budget in AGENTS.md §1 unreachable by construction.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver, Sender, TrySendError};
use vrecnu_capture::{
    CaptureBackend, CaptureError, FramePool, FrameRef, FrameSink, PaceDecision, Pacer, PooledFrame,
};
use vrecnu_encode::{live_encode_args, FfmpegProcess, LiveEncodeParams};
use vrecnu_types::{CaptureSource, CaptureSourceId, CaptureStats, Size};

use crate::audio_pipeline::{self, AudioPipeline, AudioSelection};
use crate::EngineError;

/// Buffers reserved for frames in flight.
///
/// Six is two hundred milliseconds of headroom at 30 fps — enough to ride out a scheduler stall
/// or a slow encoder hiccup, and 50 MB at 1080p, which is as much as is defensible against the
/// memory budget. Exhaustion is the backpressure signal, not a reason to allocate.
const POOL_FRAMES: usize = 6;

/// How long to wait for the very first frame before declaring the source dead.
///
/// Generous on purpose: a monitor that is genuinely idle still delivers a frame promptly, but a
/// cold GPU driver on the first capture of a session can take a second or more.
const FIRST_FRAME_TIMEOUT: Duration = Duration::from_secs(5);

/// Below this, the writer thread spins instead of sleeping.
///
/// `thread::sleep` on Windows is high-resolution but not exact, and a 33 ms cadence has no room
/// for a 15 ms overshoot. Spinning the last two milliseconds costs a few percent of one core and
/// removes the jitter entirely.
const SPIN_THRESHOLD: Duration = Duration::from_millis(2);

/// What to record and where to put it.
#[derive(Debug, Clone)]
pub struct RecordRequest {
    /// Which screen or window. `None` means the primary display.
    pub source: Option<CaptureSourceId>,
    /// Target frame rate.
    pub fps: u32,
    /// Target video bitrate in bits per second.
    pub video_bitrate: u32,
    /// Where to write the fragmented MP4.
    pub output: PathBuf,
    /// Absolute path to `ffmpeg.exe`, resolved by the shell (`ShellServices::ffmpeg_path`).
    pub ffmpeg: PathBuf,
    /// Stop after this long. `None` records until the stop flag is set.
    pub duration: Option<Duration>,
    /// Capture the microphone.
    pub mic: bool,
    /// Capture system audio via WASAPI loopback.
    pub system: bool,
}

impl RecordRequest {
    /// A 1080p30-shaped request writing to `output`.
    #[must_use]
    pub fn new(output: PathBuf, ffmpeg: PathBuf) -> Self {
        Self {
            source: None,
            fps: 30,
            video_bitrate: 6_000_000,
            output,
            ffmpeg,
            duration: None,
            mic: false,
            system: false,
        }
    }
}

/// What a finished recording amounted to.
#[derive(Debug, Clone, PartialEq)]
pub struct RecordOutcome {
    /// The file that was written.
    pub output: PathBuf,
    /// Capture resolution, as delivered rather than as reported by the OS.
    pub size: Size,
    /// Frame counters.
    pub stats: CaptureStats,
    /// Wall-clock time from the first frame to the last.
    pub elapsed: Duration,
}

/// Counters shared between the capture thread and the writer loop.
#[derive(Debug, Default)]
struct Counters {
    frames_in: AtomicU64,
    frames_dropped: AtomicU64,
    /// The size the source changed to, packed `width << 32 | height`; zero means no change.
    ///
    /// Packed into one atomic rather than two so the writer loop can never read a width from
    /// before the change alongside a height from after it, and reported rather than merely
    /// counted because the alternative — dropping the frames and carrying on — is a recording
    /// that freezes on its last good frame and says nothing.
    resized_to: AtomicU64,
}

impl Counters {
    fn record_resize(&self, size: Size) {
        let packed = (u64::from(size.width) << 32) | u64::from(size.height);
        self.resized_to.store(packed, Ordering::Relaxed);
    }

    fn resized_to(&self) -> Option<Size> {
        let packed = self.resized_to.load(Ordering::Relaxed);
        (packed != 0).then_some(Size {
            width: (packed >> 32) as u32,
            height: (packed & 0xFFFF_FFFF) as u32,
        })
    }
}

/// One frame copied out of the capture surface into a buffer we own.
struct CapturedFrame {
    size: Size,
    buffer: PooledFrame,
}

/// Copies frames off the capture thread and onto a channel, and nothing else.
///
/// Everything here runs inside the OS capture callback, which must return in about two
/// milliseconds — Windows Graphics Capture drops frames itself when it is slow. So: one
/// row-by-row memcpy into a pooled buffer, one non-blocking send, return.
struct PipelineSink {
    pool: Option<FramePool>,
    size: Option<Size>,
    tx: Sender<CapturedFrame>,
    counters: Arc<Counters>,
}

impl FrameSink for PipelineSink {
    fn on_frame(&mut self, frame: FrameRef<'_>) {
        self.counters.frames_in.fetch_add(1, Ordering::Relaxed);
        let size = Size {
            width: frame.width,
            height: frame.height,
        };

        match self.size {
            // The pool is sized from the pixels rather than from the monitor, and allocated here
            // rather than up front for exactly that reason: this is the first place the true
            // frame size is known.
            None => {
                self.size = Some(size);
                self.pool = Some(FramePool::new(size.bgra_byte_len(), POOL_FRAMES));
            }
            Some(current) if current != size => {
                // `-video_size` is already fixed. Forwarding a differently-sized frame would
                // produce a diagonally smeared image with no error anywhere, so it is dropped
                // and the writer loop is told to fail the recording rather than freeze on the
                // last good frame.
                self.counters.frames_dropped.fetch_add(1, Ordering::Relaxed);
                self.counters.record_resize(size);
                return;
            }
            Some(_) => {}
        }

        let pool = self.pool.as_ref().expect("pool exists once size is known");
        let Some(mut buffer) = pool.take() else {
            // Every buffer is in flight: the encoder is behind. Allocating instead would turn a
            // stall into an out-of-memory kill in about twenty seconds.
            self.counters.frames_dropped.fetch_add(1, Ordering::Relaxed);
            return;
        };

        copy_unstrided(frame.data, frame.stride, buffer.as_mut_slice(), size);

        if let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) =
            self.tx.try_send(CapturedFrame { size, buffer })
        {
            // The buffer rides back inside the error and returns to the pool as it drops.
            self.counters.frames_dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// Copies a stride-padded capture surface into a tightly packed BGRA buffer.
///
/// The row loop is the point. A capture surface's `stride` is alignment-padded and is *not*
/// `width * 4`; copying the whole thing in one `memcpy` skews every row by the padding and
/// produces an image that leans diagonally further with each scanline.
fn copy_unstrided(src: &[u8], stride: usize, dst: &mut [u8], size: Size) {
    let row_bytes = size.width as usize * 4;
    if stride == row_bytes && src.len() >= dst.len() {
        dst.copy_from_slice(&src[..dst.len()]);
        return;
    }
    for row in 0..size.height as usize {
        let (src_start, dst_start) = (row * stride, row * row_bytes);
        let (Some(src_row), Some(dst_row)) = (
            src.get(src_start..src_start + row_bytes),
            dst.get_mut(dst_start..dst_start + row_bytes),
        ) else {
            // A short final row means the surface is smaller than advertised; stopping leaves
            // the tail of the previous frame in place, which is far less alarming than a panic
            // on the capture thread (`panic = "abort"` would take the whole recording with it).
            break;
        };
        dst_row.copy_from_slice(src_row);
    }
}

/// Screens and windows available to capture.
///
/// # Errors
/// [`EngineError::Capture`] when the platform has no capture backend.
pub fn list_capture_sources() -> Result<Vec<CaptureSource>, EngineError> {
    let backend = new_backend()?;
    Ok(backend.enumerate()?)
}

/// Which H.264 encoder an FFmpeg build offers, for a capability report (ADR-007).
///
/// Exposed from the engine rather than letting shells call `vrecnu-encode` directly: a shell
/// that reaches past the engine for one convenient function is how the boundary in ADR-006
/// erodes, and it never erodes in an obvious place.
///
/// # Errors
/// [`EngineError::Encode`] when the binary will not run or offers neither candidate.
pub fn probe_encoder(ffmpeg: &std::path::Path) -> Result<&'static str, EngineError> {
    Ok(vrecnu_encode::probe_h264_encoder(ffmpeg)?.ffmpeg_name())
}

#[cfg(windows)]
fn new_backend() -> Result<vrecnu_capture::WgcBackend, EngineError> {
    Ok(vrecnu_capture::WgcBackend::new())
}

#[cfg(not(windows))]
fn new_backend() -> Result<NoBackend, EngineError> {
    Err(EngineError::Capture(CaptureError::Unavailable))
}

/// Placeholder so the non-Windows build still type-checks against [`CaptureBackend`].
#[cfg(not(windows))]
pub struct NoBackend;

#[cfg(not(windows))]
impl CaptureBackend for NoBackend {
    fn enumerate(&self) -> Result<Vec<CaptureSource>, CaptureError> {
        Err(CaptureError::Unavailable)
    }
    fn start(
        &mut self,
        _source: &CaptureSourceId,
        _sink: Box<dyn FrameSink>,
    ) -> Result<(), CaptureError> {
        Err(CaptureError::Unavailable)
    }
    fn stop(&mut self) -> Result<(), CaptureError> {
        Ok(())
    }
    fn frame_size(&self) -> Option<Size> {
        None
    }
}

/// Resolves the requested source, defaulting to the primary display.
fn resolve_source(request: &RecordRequest) -> Result<CaptureSourceId, EngineError> {
    match &request.source {
        Some(id) => Ok(id.clone()),
        #[cfg(windows)]
        None => Ok(vrecnu_capture::primary_monitor_source_id()?),
        #[cfg(not(windows))]
        None => Err(EngineError::Capture(CaptureError::Unavailable)),
    }
}

/// Records until `stop` is set or the requested duration elapses.
///
/// Blocks the calling thread for the whole recording. The Tauri shell will call this on a worker
/// and observe progress through `observer`; `vrecnu-cli record` calls it directly.
///
/// # Errors
/// [`EngineError::Capture`] if the source never delivers, [`EngineError::Encode`] if FFmpeg
/// cannot be started or dies mid-recording.
pub fn record(
    request: &RecordRequest,
    stop: &AtomicBool,
    observer: &mut dyn FnMut(&CaptureStats, Duration),
) -> Result<RecordOutcome, EngineError> {
    assert!(request.fps > 0, "fps must be non-zero");

    // Probing before capture starts, not after: `h264_mf` is listed in every Windows FFmpeg
    // build but absent on machines with no Media Foundation encoder, and finding that out after
    // the user pressed record means losing the recording to learn it.
    let encoder = vrecnu_encode::probe_h264_encoder(&request.ffmpeg)?;
    let source = resolve_source(request)?;

    let counters = Arc::new(Counters::default());
    let (tx, rx) = bounded::<CapturedFrame>(POOL_FRAMES);
    let mut backend = new_backend()?;
    backend.start(
        &source,
        Box::new(PipelineSink {
            pool: None,
            size: None,
            tx,
            counters: Arc::clone(&counters),
        }),
    )?;

    let result = run_until_stopped(request, stop, observer, &rx, &counters, encoder);

    // Stop capture before dealing with the encoder either way: leaving a WGC session alive after
    // a failure keeps the source's frame pool allocated for the life of the process.
    let stop_result = backend.stop();
    let outcome = result?;
    stop_result?;
    Ok(outcome)
}

/// The writer loop. Split out so `record` can always stop the backend, even on the error path.
fn run_until_stopped(
    request: &RecordRequest,
    stop: &AtomicBool,
    observer: &mut dyn FnMut(&CaptureStats, Duration),
    rx: &Receiver<CapturedFrame>,
    counters: &Counters,
    encoder: vrecnu_encode::H264Encoder,
) -> Result<RecordOutcome, EngineError> {
    let first = rx.recv_timeout(FIRST_FRAME_TIMEOUT).map_err(|_| {
        EngineError::Capture(CaptureError::NoFrames {
            after_ms: u64::try_from(FIRST_FRAME_TIMEOUT.as_millis()).unwrap_or(u64::MAX),
        })
    })?;
    let size = first.size;
    let mut current = Some(first.buffer);

    // Declared before anything fallible below: `AudioPipeline`'s `Drop` stops WASAPI capture,
    // the writer thread, and the pipe on every exit from this function — success, an `?`-early
    // return from spawning FFmpeg, all of it — with no manual cleanup threaded through each
    // return point. See `audio_pipeline::AudioPipeline::stop`'s doc comment.
    //
    // Started before FFmpeg, not after: the pipe has to exist for FFmpeg's `-i` to open, and a
    // bad device should be reported before a recording has even begun rather than partway in.
    let mut audio = audio_pipeline::start(AudioSelection {
        mic: request.mic,
        system: request.system,
    })?;
    let audio_pipe_path = audio.as_ref().map(AudioPipeline::pipe_path);

    let args = live_encode_args(&LiveEncodeParams {
        size,
        fps: request.fps,
        video_bitrate: request.video_bitrate,
        encoder,
        audio_pipe: audio_pipe_path.as_deref(),
        camera_pipe: None,
        camera_size: None,
        output: &request.output,
    });
    tracing::info!(
        size = format!("{}x{}", size.width, size.height),
        fps = request.fps,
        encoder = encoder.ffmpeg_name(),
        audio = audio_pipe_path.is_some(),
        "starting live encode"
    );
    let mut ffmpeg = FfmpegProcess::spawn(&request.ffmpeg, &args)?;

    // Everything above happened while frames were already being captured, so the clock starts
    // here — the moment the first tick can actually be written. Audio's clock is this same
    // instant, not one it measures itself — see the audio_pipeline module doc comment for why.
    let started = Instant::now();
    if let Some(audio) = audio.as_mut() {
        audio.begin(started);
    }
    let mut pacer = Pacer::new(request.fps, 0);
    let max_frames = request
        .duration
        .map(|duration| (duration.as_secs_f64() * f64::from(request.fps)).round() as u64);

    let mut last_report = started;

    let outcome = loop {
        if let Some(max) = max_frames {
            if pacer.frames_sent() >= max {
                break Ok(());
            }
        }
        if stop.load(Ordering::Relaxed) {
            break Ok(());
        }

        // Absolute deadline. Deriving it from `now` instead would fold every oversleep into the
        // timeline and speed the finished video up.
        sleep_until(started + Duration::from_nanos(pacer.next_tick_ns()));

        if let Some(new_size) = counters.resized_to() {
            break Err(EngineError::Capture(CaptureError::SizeChanged {
                from_w: size.width,
                from_h: size.height,
                to_w: new_size.width,
                to_h: new_size.height,
            }));
        }

        let mut arrived = false;
        while let Ok(frame) = rx.try_recv() {
            // The sink only forwards frames matching the size FFmpeg was started with, so
            // keeping the newest here is purely the pacer doing its job on a 60 Hz screen
            // recorded at 30 fps; superseded buffers return to the pool as they drop.
            debug_assert_eq!(frame.size, size);
            current = Some(frame.buffer);
            arrived = true;
        }

        match pacer.tick(arrived) {
            PaceDecision::SendNew | PaceDecision::Duplicate => {
                if let Some(frame) = current.as_ref() {
                    if let Err(err) = ffmpeg.write_frame(frame.as_slice()) {
                        break Err(EngineError::from(err));
                    }
                }
            }
            // Unreachable in practice: `current` is populated before the loop starts. Kept
            // because the pacer's contract, not this loop's shape, is what guarantees it.
            PaceDecision::Skip => {}
        }

        if last_report.elapsed() >= Duration::from_secs(1) {
            last_report = Instant::now();
            observer(
                &collect_stats(counters, &pacer, ffmpeg.progress()),
                started.elapsed(),
            );
        }
    };

    let elapsed = started.elapsed();

    if let Err(err) = outcome {
        return Err(match err {
            // A broken pipe only says our end noticed. FFmpeg's stderr says why.
            EngineError::Encode(encode) => EngineError::Encode(ffmpeg.failure_reason(encode)),
            other => {
                ffmpeg.abort();
                other
            }
        });
    }

    // Audio's EOF has to land *before* we wait for FFmpeg to exit, not after. `ffmpeg.finish()`
    // only closes the video stdin; a muxer reading two live inputs has no reason to finalize and
    // exit while the audio pipe is still open and being actively written to. Stopping it here —
    // flush, disconnect, close, all inside `AudioPipeline::stop` — is audio's side of exactly
    // the same "close rather than kill" finalization `ffmpeg.finish()` already does for video.
    // Found by running it: without this, `finish()` never returned — not a deadlock inside
    // FFmpeg or this process, just two inputs each waiting on something only the other one
    // could have provided.
    if let Some(audio) = audio.as_mut() {
        audio.stop();
    }

    // Closing stdin rather than killing: FFmpeg flushes the final fragment and exits 0. Killing
    // here would cost the user the last couple of seconds of what they just recorded.
    let progress = ffmpeg.finish()?;
    let stats = collect_stats(counters, &pacer, progress);

    tracing::info!(
        frames_written = stats.frames_written,
        frames_duplicated = stats.frames_duplicated,
        frames_dropped = stats.frames_dropped,
        elapsed_ms = elapsed.as_millis(),
        "recording finished"
    );

    Ok(RecordOutcome {
        output: request.output.clone(),
        size,
        stats,
        elapsed,
    })
}

fn collect_stats(
    counters: &Counters,
    pacer: &Pacer,
    progress: vrecnu_encode::FfmpegProgress,
) -> CaptureStats {
    CaptureStats {
        frames_in: counters.frames_in.load(Ordering::Relaxed),
        frames_written: pacer.frames_sent(),
        // FFmpeg's own `drop_frames` counts the same failure from the far side of the pipe, so
        // both are reported rather than only the one we happen to observe first.
        frames_dropped: counters.frames_dropped.load(Ordering::Relaxed) + progress.drop_frames,
        frames_duplicated: pacer.frames_duplicated(),
        encoder_fps: progress.fps,
    }
}

/// Waits until `target`, sleeping most of the way and spinning the last couple of milliseconds.
///
/// A pure `thread::sleep` misses a 33 ms cadence often enough to matter even with the
/// high-resolution timer std uses on Windows, and the miss is not self-correcting when it lands
/// on the wrong side of a frame boundary.
pub(crate) fn sleep_until(target: Instant) {
    loop {
        let now = Instant::now();
        if now >= target {
            return;
        }
        let remaining = target - now;
        if remaining > SPIN_THRESHOLD {
            std::thread::sleep(remaining - SPIN_THRESHOLD);
        } else {
            std::hint::spin_loop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(width: u32, height: u32) -> Size {
        Size { width, height }
    }

    #[test]
    fn a_padded_surface_is_copied_row_by_row() {
        // The failure this prevents is the classic diagonal smear: a capture surface's stride is
        // alignment-padded, and one flat memcpy shifts every row by the padding.
        let target = size(2, 2);
        let stride = 16; // 8 bytes of real pixels, 8 bytes of padding
        let src = vec![
            1, 1, 1, 1, 2, 2, 2, 2, 9, 9, 9, 9, 9, 9, 9, 9, // row 0 + padding
            3, 3, 3, 3, 4, 4, 4, 4, 9, 9, 9, 9, 9, 9, 9, 9, // row 1 + padding
        ];
        let mut dst = vec![0u8; target.bgra_byte_len()];
        copy_unstrided(&src, stride, &mut dst, target);
        assert_eq!(dst, vec![1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4, 4]);
    }

    #[test]
    fn an_unpadded_surface_takes_the_flat_copy_path() {
        let target = size(2, 1);
        let src = vec![1, 2, 3, 4, 5, 6, 7, 8];
        let mut dst = vec![0u8; target.bgra_byte_len()];
        copy_unstrided(&src, 8, &mut dst, target);
        assert_eq!(dst, src);
    }

    #[test]
    fn a_truncated_surface_stops_instead_of_panicking() {
        // `panic = "abort"` in release means a panic on the capture thread takes the recording
        // with it, so a short buffer must degrade rather than unwind.
        let target = size(2, 4);
        let src = vec![7u8; 8]; // one row's worth for a four-row frame
        let mut dst = vec![0u8; target.bgra_byte_len()];
        copy_unstrided(&src, 8, &mut dst, target);
        assert_eq!(&dst[..8], &[7u8; 8]);
        assert_eq!(&dst[8..], &[0u8; 24]);
    }

    #[test]
    fn the_sink_drops_rather_than_allocating_when_the_pool_is_empty() {
        // Exhaustion is the backpressure signal. Growing the pool here is a slow OOM.
        let counters = Arc::new(Counters::default());
        let (tx, rx) = bounded::<CapturedFrame>(POOL_FRAMES);
        let mut sink = PipelineSink {
            pool: None,
            size: None,
            tx,
            counters: Arc::clone(&counters),
        };
        let frame = vec![0u8; 16];
        for _ in 0..(POOL_FRAMES + 4) {
            sink.on_frame(FrameRef {
                data: &frame,
                stride: 8,
                width: 2,
                height: 2,
                qpc: 0,
            });
        }
        assert_eq!(
            counters.frames_in.load(Ordering::Relaxed),
            POOL_FRAMES as u64 + 4
        );
        assert_eq!(counters.frames_dropped.load(Ordering::Relaxed), 4);
        assert_eq!(rx.len(), POOL_FRAMES);
    }

    #[test]
    fn a_resolution_change_is_dropped_rather_than_smeared_into_the_file() {
        // `-video_size` is fixed at spawn; a differently-sized frame cannot be absorbed.
        let counters = Arc::new(Counters::default());
        let (tx, _rx) = bounded::<CapturedFrame>(POOL_FRAMES);
        let mut sink = PipelineSink {
            pool: None,
            size: None,
            tx,
            counters: Arc::clone(&counters),
        };
        let frame = vec![0u8; 64];
        sink.on_frame(FrameRef {
            data: &frame,
            stride: 8,
            width: 2,
            height: 2,
            qpc: 0,
        });
        sink.on_frame(FrameRef {
            data: &frame,
            stride: 16,
            width: 4,
            height: 4,
            qpc: 0,
        });
        assert_eq!(sink.size, Some(size(2, 2)));
        assert_eq!(counters.frames_dropped.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn buffers_return_to_the_pool_as_the_writer_consumes_them() {
        let counters = Arc::new(Counters::default());
        let (tx, rx) = bounded::<CapturedFrame>(POOL_FRAMES);
        let mut sink = PipelineSink {
            pool: None,
            size: None,
            tx,
            counters,
        };
        let frame = vec![0u8; 16];
        for _ in 0..(POOL_FRAMES * 3) {
            sink.on_frame(FrameRef {
                data: &frame,
                stride: 8,
                width: 2,
                height: 2,
                qpc: 0,
            });
            // Draining each time is what the writer loop does every tick.
            drop(rx.try_recv());
        }
        assert_eq!(sink.pool.as_ref().unwrap().capacity(), POOL_FRAMES);
    }

    #[test]
    fn a_request_defaults_to_1080p30_shaped_settings() {
        let request = RecordRequest::new(PathBuf::from("out.mp4"), PathBuf::from("ffmpeg.exe"));
        assert_eq!(request.fps, 30);
        assert!(request.source.is_none());
        assert!(request.duration.is_none());
    }

    #[test]
    fn sleeping_to_a_deadline_that_has_passed_returns_immediately() {
        let start = Instant::now();
        sleep_until(start - Duration::from_secs(1));
        assert!(start.elapsed() < Duration::from_millis(50));
    }

    #[test]
    fn sleeping_to_a_deadline_lands_close_to_it() {
        // The whole timeline depends on this: a 33 ms cadence has no room for a 15 ms overshoot.
        let start = Instant::now();
        let target = start + Duration::from_millis(40);
        sleep_until(target);
        let overshoot = Instant::now().saturating_duration_since(target);
        assert!(
            overshoot < Duration::from_millis(5),
            "overshot by {overshoot:?}"
        );
    }

    #[test]
    fn stats_combine_both_sides_of_the_pipe() {
        // FFmpeg counts drops from its end of the pipe and the sink counts them from ours;
        // reporting only one hides half the failure.
        let counters = Counters::default();
        counters.frames_in.store(310, Ordering::Relaxed);
        counters.frames_dropped.store(2, Ordering::Relaxed);
        let mut pacer = Pacer::new(30, 0);
        pacer.tick(true);
        pacer.tick(false);
        let stats = collect_stats(
            &counters,
            &pacer,
            vrecnu_encode::FfmpegProgress {
                drop_frames: 3,
                fps: 29.5,
                ..vrecnu_encode::FfmpegProgress::default()
            },
        );
        assert_eq!(stats.frames_in, 310);
        assert_eq!(stats.frames_written, 2);
        assert_eq!(stats.frames_duplicated, 1);
        assert_eq!(stats.frames_dropped, 5);
    }
}
