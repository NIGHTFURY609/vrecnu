//! Mixed audio → named pipe, on the recording's shared wall clock.
//!
//! Mirrors [`crate::pipeline`]'s shape deliberately: an OS capture callback that must not block,
//! delivered on its own thread, drained by a writer loop that ticks on absolute deadlines. The
//! one property specific to audio is that **its clock is never its own** — `started` is set once,
//! by the first video frame (see `pipeline::run_until_stopped`), and handed to this module via
//! [`AudioPipeline::begin`] rather than measured independently. Two clocks that each start at
//! their own "first sample" is exactly the drift bug `vrecnu-audio::SilenceFiller` exists to
//! prevent, and it only works if both sides tick against the same zero.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::EngineError;

/// How often the writer thread ticks the mixer and writes to the pipe.
///
/// Small relative to a video frame interval (33 ms at 30 fps) so audio never visibly lags
/// picture; large enough that the per-tick byte count comfortably clears one WASAPI period.
const TICK_INTERVAL: Duration = Duration::from_millis(20);

/// How long [`AudioPipeline::stop`] waits for the writer thread before giving up on it.
///
/// The only way this bound is ever hit is FFmpeg dying between process spawn and opening its
/// audio input — vanishingly rare, and already a failed recording by the time it happens (the
/// video writer loop finds the same dead process independently, from its own stdin write). This
/// bound exists so that pathological case costs the caller three seconds, not a permanent hang;
/// the accept thread itself is abandoned rather than force-cancelled (`ConnectNamedPipe` has no
/// portable cancel-from-another-thread short of overlapped I/O, which this module does not use)
/// and is reclaimed when the process exits.
const STOP_GRACE: Duration = Duration::from_secs(3);

static PIPE_COUNTER: AtomicU32 = AtomicU32::new(0);

fn unique_pipe_name() -> String {
    let n = PIPE_COUNTER.fetch_add(1, Ordering::Relaxed);
    format!("vrecnu-audio-{}-{n}", std::process::id())
}

/// A live audio pipeline: WASAPI source(s), a mixer, and the named pipe FFmpeg reads from.
///
/// Constructed by [`start`] before FFmpeg is spawned (so the pipe exists for FFmpeg's `-i` to
/// open) and told the recording's clock via [`begin`] once the video pacer has one. `Drop` stops
/// everything — capture, the writer thread, and the pipe — so simply letting this go out of
/// scope is the intended shutdown, not a fallback; see [`stop`]'s doc comment for why.
///
/// [`start`]: AudioPipeline::start
/// [`begin`]: AudioPipeline::begin
/// [`stop`]: AudioPipeline::stop
pub struct AudioPipeline {
    pipe_name: String,
    stop_flag: Arc<AtomicBool>,
    start_tx: Option<mpsc::Sender<Instant>>,
    writer: Option<JoinHandle<()>>,
    sources: Vec<Box<dyn imp::AudioSourceHandle>>,
}

impl AudioPipeline {
    /// The full `\\.\pipe\...` path — pass this as `LiveEncodeParams::audio_pipe`.
    #[must_use]
    pub fn pipe_path(&self) -> String {
        vrecnu_namedpipe::pipe_path(&self.pipe_name)
    }

    /// Unblocks the writer thread's tick loop, anchored to `started`.
    ///
    /// Call this exactly once, right after the video pacer sets its own `started` — see the
    /// module doc comment for why sharing one instant, rather than each side measuring its own,
    /// is the entire point of this module.
    pub fn begin(&mut self, started: Instant) {
        if let Some(tx) = self.start_tx.take() {
            // An error here means the writer thread already exited (most likely FFmpeg never
            // opened the audio pipe) — nothing to unblock, and `stop()` will report the same
            // condition through its own bounded wait rather than this call surfacing it twice.
            let _ = tx.send(started);
        }
    }

    /// Stops WASAPI capture, then the writer thread, then drops the pipe server — which flushes
    /// and closes it, giving FFmpeg's audio input a clean EOF, the audio-side equivalent of
    /// `FfmpegProcess::finish` closing stdin for video.
    ///
    /// Idempotent — safe to call explicitly and still let `Drop` run. Takes `&mut self` rather
    /// than consuming, specifically so callers with several early-return paths (`run_until_
    /// stopped`'s `?`-heavy body) can rely on `Drop` alone: declare `audio` before anything
    /// fallible, and every exit — success, error, an early `?` from an unrelated call — stops it
    /// the same way, instead of needing a manual stop call threaded through each return point.
    ///
    /// Bounded by [`STOP_GRACE`]; see its doc comment for the one case that bound exists for.
    pub fn stop(&mut self) {
        // Sources first: no more PCM should flow into the mixer once we're shutting down,
        // regardless of how the writer thread's own shutdown goes. Each source's own `stop()`
        // is itself bounded (WASAPI's wait loop polls in `EVENT_TIMEOUT_MS`-sized slices), so
        // this never contributes to the hang `STOP_GRACE` guards against.
        for source in &mut self.sources {
            source.stop();
        }
        self.sources.clear();

        self.stop_flag.store(true, Ordering::Relaxed);
        let Some(handle) = self.writer.take() else {
            return;
        };

        let (done_tx, done_rx) = mpsc::channel::<()>();
        // The join itself happens on a throwaway thread so a writer stuck forever in
        // `ConnectNamedPipe` (no client ever showed up) costs this call `STOP_GRACE`, not the
        // rest of the process's life.
        std::thread::spawn(move || {
            let _ = handle.join();
            let _ = done_tx.send(());
        });
        if done_rx.recv_timeout(STOP_GRACE).is_err() {
            tracing::warn!(
                "audio pipeline writer thread did not stop within {STOP_GRACE:?}; abandoning it"
            );
        }
    }
}

impl Drop for AudioPipeline {
    /// This *is* the primary shutdown path, not a safety net — `run_until_stopped` never calls
    /// [`Self::stop`] itself. It declares `audio: Option<AudioPipeline>` before the first
    /// fallible call and lets scope-based drop stop it uniformly on every exit, success or any
    /// `?`-triggered early return alike, per [`Self::stop`]'s own doc comment. `stop` is still
    /// exposed as a public method for a caller that wants its outcome visible at an explicit
    /// call site; calling it there first makes this `drop` a no-op via its `writer.take()`
    /// idempotency check.
    fn drop(&mut self) {
        self.stop();
    }
}

/// Which sources to capture, or `None` for a video-only recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AudioSelection {
    /// Capture the microphone.
    pub mic: bool,
    /// Capture system audio via WASAPI loopback.
    pub system: bool,
}

impl AudioSelection {
    /// True when neither `mic` nor `system` is set — `Self::default()` is the same value; this
    /// just names the check `start()` needs at the one call site that needs it.
    const fn is_none(self) -> bool {
        !self.mic && !self.system
    }
}

/// Starts capture and the named pipe. Returns `Ok(None)` for `AudioSelection::default()` (both
/// flags unset) — a video-only recording has nothing for this module to do.
///
/// Blocks until every requested device is confirmed open — device failures surface here, before
/// FFmpeg is spawned, the same "probe before capture starts" discipline `record()` already
/// applies to the H.264 encoder probe.
///
/// # Errors
/// [`EngineError::Pipe`] if the named pipe cannot be created. [`EngineError::Audio`] if a
/// requested device cannot be opened — any source already started is stopped before returning,
/// so a mic failure after system audio already opened never leaks a running capture thread.
pub fn start(selection: AudioSelection) -> Result<Option<AudioPipeline>, EngineError> {
    if selection.is_none() {
        return Ok(None);
    }
    imp::start(selection)
}

#[cfg(windows)]
mod imp {
    use vrecnu_audio::{AudioSource, Mixer, MixerSources};
    use vrecnu_namedpipe::NamedPipeServer;

    use super::{unique_pipe_name, AudioPipeline, AudioSelection, EngineError, TICK_INTERVAL};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};
    use std::time::Instant;

    /// Erases which concrete `AudioSource` a slot holds, so [`AudioPipeline`] can hold a mic and
    /// a loopback source in the same `Vec` without a shell-visible enum.
    pub(crate) trait AudioSourceHandle: Send {
        fn stop(&mut self);
    }

    struct Started<S: AudioSource>(S);
    impl<S: AudioSource> AudioSourceHandle for Started<S> {
        fn stop(&mut self) {
            let _ = self.0.stop();
        }
    }

    fn mixer_sources(selection: AudioSelection) -> MixerSources {
        match (selection.mic, selection.system) {
            (true, true) => MixerSources::Both,
            (true, false) => MixerSources::Mic,
            (false, true) => MixerSources::System,
            (false, false) => {
                unreachable!("start() already returned early for AudioSelection::none")
            }
        }
    }

    pub(super) fn start(selection: AudioSelection) -> Result<Option<AudioPipeline>, EngineError> {
        let pipe_name = unique_pipe_name();
        let server = NamedPipeServer::create(&pipe_name)?;

        let (mixer, sinks) = Mixer::new(mixer_sources(selection));
        let mut sources: Vec<Box<dyn AudioSourceHandle>> = Vec::new();

        // Starts, and on failure stops whatever already started rather than leaking a running
        // capture thread — a mic failure after system audio already opened must not orphan it.
        macro_rules! start_source {
            ($ctor:expr, $sink:expr) => {
                if let Some(sink) = $sink {
                    let mut source = $ctor;
                    if let Err(err) = source.start(sink) {
                        for started in &mut sources {
                            started.stop();
                        }
                        return Err(EngineError::from(err));
                    }
                    sources.push(Box::new(Started(source)));
                }
            };
        }
        start_source!(vrecnu_audio::WasapiMicSource::new(), sinks.mic);
        start_source!(vrecnu_audio::WasapiLoopbackSource::new(), sinks.system);

        let stop_flag = Arc::new(AtomicBool::new(false));
        let (start_tx, start_rx) = mpsc::channel::<Instant>();
        let stop_for_writer = Arc::clone(&stop_flag);

        let writer =
            std::thread::spawn(move || writer_loop(server, mixer, &stop_for_writer, &start_rx));

        Ok(Some(AudioPipeline {
            pipe_name,
            stop_flag,
            start_tx: Some(start_tx),
            writer: Some(writer),
            sources,
        }))
    }

    fn writer_loop(
        server: NamedPipeServer,
        mut mixer: Mixer,
        stop: &AtomicBool,
        start_rx: &mpsc::Receiver<Instant>,
    ) {
        // No client yet: FFmpeg hasn't been spawned when this thread starts (it's spawned by
        // `record()` right after this function's caller returns), so this call is expected to
        // block for a real, if short, stretch of wall-clock time — that's the point, not a bug.
        if server.wait_for_client().is_err() {
            return;
        }

        // Blocks until `AudioPipeline::begin` is called, or returns without ticking anything if
        // the recording aborted before a single video frame ever arrived — there was never a
        // clock to tick against, so there is nothing to have written.
        let Ok(started) = start_rx.recv() else {
            return;
        };

        let mut next_tick = started;
        while !stop.load(Ordering::Relaxed) {
            next_tick += TICK_INTERVAL;
            crate::pipeline::sleep_until(next_tick);
            if stop.load(Ordering::Relaxed) {
                break;
            }

            let elapsed_ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);
            let bytes = mixer.tick(elapsed_ns);
            if !bytes.is_empty() && server.write_all(&bytes).is_err() {
                // FFmpeg's audio input died. The video side will independently notice its own
                // stdin break; nothing productive left for this thread to do but stop.
                break;
            }
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use super::{AudioPipeline, AudioSelection, EngineError};

    pub(super) fn start(_selection: AudioSelection) -> Result<Option<AudioPipeline>, EngineError> {
        Err(EngineError::Audio(vrecnu_audio::AudioError::Backend(
            "audio capture is only implemented on Windows".into(),
        )))
    }

    pub(crate) trait AudioSourceHandle: Send {
        fn stop(&mut self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_selection_starts_nothing() {
        let result = start(AudioSelection::default());
        assert!(matches!(result, Ok(None)));
    }

    #[test]
    fn a_selection_with_neither_flag_is_none() {
        assert!(AudioSelection::default().is_none());
        assert!(!AudioSelection {
            mic: true,
            system: false
        }
        .is_none());
    }

    #[test]
    fn pipe_names_are_unique_across_calls() {
        let a = unique_pipe_name();
        let b = unique_pipe_name();
        assert_ne!(a, b);
    }
}
