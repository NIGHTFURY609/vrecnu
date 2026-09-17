//! [`RecorderEngine`] — the engine as a shell sees it.
//!
//! Everything below `pipeline` is a free function that blocks until a recording is finished.
//! That is the right shape for `vrecnu-cli`, which has nothing else to do, and exactly the
//! wrong shape for a UI, which must stay repainting. This module is the adapter: it owns the
//! session, runs the blocking pipeline on a worker thread, and turns what the pipeline reports
//! into the coalesced event stream `vrecnu-shell-api` promises.
//!
//! **No shell may be named here.** This type is what makes ADR-006's boundary real rather than
//! aspirational: the Tauri shell, `vrecnu-cli`, and a future `egui` shell all drive the same
//! `Arc<dyn RecorderEngine>`, and none of them can tell which one the others are.
//!
//! Three rules the trait imposes, and how they are kept:
//!
//! 1. **Nothing blocks for longer than a repaint.** `start` spawns and returns; `stop` sets an
//!    atomic and returns. The only method that waits is [`Session::cancel`], and it waits on a
//!    worker that has already been told to stop.
//! 2. **Nothing is emitted per frame.** `Stats` rides the pipeline's existing ~1 Hz observer
//!    callback; `Elapsed` comes from a separate ticker so it can hit the ~4 Hz the contract
//!    asks for without making the pipeline report ten times as often.
//! 3. **A late subscriber must not miss state.** Events are fire-and-forget, so
//!    [`Session::snapshot`] is the authority and every shell reads it on connect. A webview
//!    that reloads mid-recording recovers from the snapshot alone.
//!
//! ## What is honestly not implemented yet
//!
//! Several trait methods depend on milestones that have not landed. They return an `AppError`
//! saying so rather than a plausible-looking empty success, because a shell that cannot tell
//! "no cameras" from "camera enumeration is not written yet" will ship the wrong UI for it.
//! Each one names the milestone: publishing is M5, rendering an edit is M7, and thumbnails and
//! per-device audio selection have no milestone yet.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Receiver, RecvTimeoutError, Sender};
use parking_lot::Mutex;
use vrecnu_publish_server::{PublishAssets, PublishServer, PublishSignal, RecordingToServe};
use vrecnu_shell_api::{
    AppError, AppState, EditPreview, EngineEvent, PublishHandle, PublishResult, RecorderEngine,
    Result, ShellServices, Thumbnail,
};
use vrecnu_types::{
    AudioDevice, AudioDevices, CameraInfo, CaptureSource, CaptureSourceId, CaptureStats, EditSpec,
    ErrorCode, Recording, RenderId, SessionConfig, SessionId, SessionState,
};

use crate::machine::SessionMachine;
use crate::pipeline::{self, RecordRequest};
use crate::EngineError;

/// How often the elapsed-time ticker fires while recording.
///
/// 250 ms is the ~4 Hz the shell-api contract asks for. It is deliberately *not* folded into
/// the pipeline's observer callback: that fires once a second because a second is the right
/// cadence for capture counters, and speeding it up to drive a clock would make the pipeline
/// do ten times the work to move one label.
const ELAPSED_TICK: Duration = Duration::from_millis(250);

/// How long [`Session::cancel`] waits for the worker to notice the stop flag.
///
/// The worker is already draining FFmpeg by the time this matters, and that drain is itself
/// bounded (see `pipeline`'s shutdown ordering). Waiting is still bounded here because a shell
/// calling `cancel` is on its UI thread and a hung engine must not become a hung window.
const CANCEL_GRACE: Duration = Duration::from_secs(20);

/// Engine state a shell can observe, behind one lock.
///
/// One `Mutex` rather than a lock per field, because a shell reading `snapshot()` must see a
/// *consistent* state: a `session` of `Stopped` next to a `recording` of `None` is a state the
/// engine is never actually in, and splitting the lock is how a shell would observe it anyway.
struct Inner {
    machine: SessionMachine,
    config: SessionConfig,
    session_id: Option<SessionId>,
    elapsed_ms: u64,
    stats: CaptureStats,
    recording: Option<Recording>,
    error: Option<AppError>,
}

impl Inner {
    /// `publishing` is passed in rather than stored: the publish pump thread flips it without
    /// taking this lock, and a shell must never see `publishing: true` next to a stale session
    /// state because the two were read at different moments.
    fn snapshot(&self, publishing: bool) -> AppState {
        AppState {
            session: self.machine.state(),
            session_id: self.session_id,
            elapsed_ms: self.elapsed_ms,
            stats: self.stats,
            recording: self.recording.clone(),
            error: self.error.clone(),
            publishing,
        }
    }
}

/// The engine, as a shell holds it.
///
/// Construct one per process and share it: `Arc<Session> as Arc<dyn RecorderEngine>`.
pub struct Session {
    shell: Arc<dyn ShellServices>,
    inner: Mutex<Inner>,
    /// Set to stop the worker. Replaced per session rather than reset, so a stale worker from a
    /// cancelled session can never see a fresh session's flag.
    stop: Mutex<Arc<AtomicBool>>,
    worker: Mutex<Option<JoinHandle<()>>>,
    /// `Arc` so the publish pump thread can emit without holding a reference to the session.
    /// The alternative -- handing the pump a `Weak<Session>` -- would make `Session` only
    /// constructible inside an `Arc`, which is a constraint the trait does not impose.
    subscribers: Arc<Mutex<Vec<Sender<EngineEvent>>>>,
    publish: Mutex<Option<PublishPump>>,
    publishing: Arc<AtomicBool>,
    next_session: AtomicU64,
}

/// The thread that turns loopback-server signals into engine events.
///
/// It **owns** the `PublishServer`, so the listening port is released exactly when this thread
/// exits and there is no second place that could forget to release it.
struct PublishPump {
    stop: Arc<AtomicBool>,
    join: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let inner = self.inner.lock();
        f.debug_struct("Session")
            .field("state", &inner.machine.state())
            .field("session_id", &inner.session_id)
            .field("subscribers", &self.subscribers.lock().len())
            .finish()
    }
}

impl Session {
    /// A new, idle session bound to `shell`.
    #[must_use]
    pub fn new(shell: Arc<dyn ShellServices>) -> Self {
        Self {
            shell,
            inner: Mutex::new(Inner {
                machine: SessionMachine::new(),
                config: SessionConfig::default(),
                session_id: None,
                elapsed_ms: 0,
                stats: CaptureStats::default(),
                recording: None,
                error: None,
            }),
            stop: Mutex::new(Arc::new(AtomicBool::new(false))),
            worker: Mutex::new(None),
            subscribers: Arc::new(Mutex::new(Vec::new())),
            publish: Mutex::new(None),
            publishing: Arc::new(AtomicBool::new(false)),
            next_session: AtomicU64::new(1),
        }
    }

    /// Points the session at a recording this process did not make.
    ///
    /// Not on [`RecorderEngine`]: a shell driving a live session never needs it. It exists for
    /// the case where the recording and the publish are separate runs -- `vrecnu-cli record`
    /// then `vrecnu-cli publish`, and later "publish something from your recordings folder" --
    /// which would otherwise force a re-record to exercise the Drive half of the product.
    ///
    /// Deliberately leaves the state machine alone. This is not a session that happened; it is
    /// a file that exists. Reporting `Stopped` would tell a shell there is an elapsed time and
    /// a set of capture stats to render, and there are neither.
    ///
    /// # Errors
    ///
    /// If the file cannot be read.
    pub fn adopt_recording(&self, path: &Path) -> Result<Recording> {
        let meta = std::fs::metadata(path).map_err(|err| {
            AppError::recoverable(ErrorCode::IllegalState, "That recording could not be read.")
                .with_detail(err.to_string())
        })?;
        let recording = Recording {
            session: SessionId(0),
            path: path.to_path_buf(),
            // Unknown rather than guessed. `begin_publish` turns a zero into the page's
            // "Drive's preview may not seek" warning, which is exactly the right thing to say
            // about a file we have not probed.
            duration_ms: 0,
            size_bytes: meta.len(),
            mime_type: mime_for_extension(path),
        };
        self.inner.lock().recording = Some(recording.clone());
        Ok(recording)
    }

    /// Fans one event out, dropping subscribers whose receiver has gone.
    ///
    /// Fire-and-forget by design. A shell that missed an event re-reads [`Self::snapshot`];
    /// blocking the engine because a webview stopped draining its channel would let a stalled
    /// UI stall a recording.
    fn emit(&self, event: &EngineEvent) {
        self.subscribers
            .lock()
            .retain(|tx| tx.send(event.clone()).is_ok());
    }

    /// Moves the machine and announces it in one step, so no shell can observe a state change
    /// that was never emitted.
    fn transition(&self, inner: &mut Inner, to: SessionState) {
        let from = inner.machine.state();
        if from == to {
            return;
        }
        let moved = match to {
            SessionState::Ready => inner.machine.arm().is_ok(),
            SessionState::Recording => inner.machine.start().is_ok(),
            SessionState::Finalizing => inner.machine.stop().is_ok(),
            SessionState::Stopped => inner.machine.finalized().is_ok(),
            SessionState::Failed => {
                inner.machine.fail();
                true
            }
            SessionState::Idle => {
                inner.machine.reset();
                true
            }
        };
        if moved {
            self.emit(&EngineEvent::StateChanged { from, to });
        }
    }

    /// Turns an engine failure into the flat error a shell sees, records it, and fails the
    /// session. The one place a running session becomes a failed one.
    fn fail(&self, err: EngineError) -> AppError {
        let app = AppError::from(err);
        {
            let mut inner = self.inner.lock();
            inner.error = Some(app.clone());
            let from = inner.machine.state();
            inner.machine.fail();
            if from != SessionState::Failed {
                self.emit(&EngineEvent::StateChanged {
                    from,
                    to: SessionState::Failed,
                });
            }
        }
        self.emit(&EngineEvent::Failed { error: app.clone() });
        app
    }

    /// Where this session's capture is written.
    fn output_path(&self, session: SessionId) -> Result<PathBuf> {
        let dir = self.shell.data_dir()?;
        Ok(dir.join(format!("vrecnu-{}.mp4", session.0)))
    }

    /// Builds the pipeline request from the stored configuration.
    ///
    /// The one place `SessionConfig` (what a shell chose) becomes `RecordRequest` (what the
    /// pipeline needs). Keeping the translation here rather than in the pipeline is what lets
    /// `vrecnu-cli` build a `RecordRequest` directly without inventing a `SessionConfig`.
    fn build_request(&self, config: &SessionConfig, output: PathBuf) -> Result<RecordRequest> {
        let ffmpeg = self.shell.ffmpeg_path()?;
        Ok(RecordRequest {
            source: config.source.clone(),
            fps: config.fps,
            video_bitrate: config.video_bitrate,
            output,
            ffmpeg,
            // A UI stops on a button press, not on a timer. The pipeline's duration cap exists
            // for `vrecnu-cli --seconds`, which is a different caller.
            duration: None,
            mic: config.microphone.is_some(),
            system: config.system_audio,
            camera: config.camera.clone(),
            bubble: config.bubble,
        })
    }
}

/// Everything the worker thread needs, so spawning does not borrow `self`.
struct WorkerContext {
    session: Arc<Session>,
    request: RecordRequest,
    stop: Arc<AtomicBool>,
    id: SessionId,
}

/// Runs one recording to completion and settles the session around it.
fn run_worker(ctx: WorkerContext) {
    let WorkerContext {
        session,
        request,
        stop,
        id,
    } = ctx;

    // The elapsed ticker. Separate from the pipeline's observer so the clock can tick at ~4 Hz
    // without asking the pipeline to report its counters four times as often.
    let ticker_stop = Arc::clone(&stop);
    let ticker_session = Arc::clone(&session);
    let started = Instant::now();
    let ticker = std::thread::spawn(move || {
        while !ticker_stop.load(Ordering::Relaxed) {
            std::thread::sleep(ELAPSED_TICK);
            if ticker_stop.load(Ordering::Relaxed) {
                break;
            }
            let ms = u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
            ticker_session.inner.lock().elapsed_ms = ms;
            ticker_session.emit(&EngineEvent::Elapsed { ms });
        }
    });

    let observer_session = Arc::clone(&session);
    let mut observer = move |stats: &CaptureStats, elapsed: Duration| {
        {
            let mut inner = observer_session.inner.lock();
            inner.stats = *stats;
            inner.elapsed_ms = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        }
        observer_session.emit(&EngineEvent::Stats(*stats));
    };

    let outcome = pipeline::record(&request, &stop, &mut observer);

    // Stop the ticker before settling state, so no `Elapsed` can arrive after `StateChanged`
    // and make a stopped recording look like it is still counting up.
    stop.store(true, Ordering::Relaxed);
    let _ = ticker.join();

    match outcome {
        Ok(result) => {
            let size_bytes = std::fs::metadata(&result.output).map_or(0, |meta| meta.len());
            let recording = Recording {
                session: id,
                path: result.output.clone(),
                duration_ms: u64::try_from(result.elapsed.as_millis()).unwrap_or(u64::MAX),
                size_bytes,
                mime_type: "video/mp4".to_string(),
            };

            let mut inner = session.inner.lock();
            inner.stats = result.stats;
            inner.elapsed_ms = recording.duration_ms;
            inner.recording = Some(recording);
            // `stop()` may already have moved the machine to Finalizing; a duration-capped or
            // self-terminating run reaches here still in Recording. Both must end in Stopped.
            if inner.machine.state() == SessionState::Recording {
                session.transition(&mut inner, SessionState::Finalizing);
            }
            session.transition(&mut inner, SessionState::Stopped);
        }
        Err(err) => {
            session.fail(err);
        }
    }
}

impl RecorderEngine for Session {
    fn list_capture_sources(&self) -> Result<Vec<CaptureSource>> {
        pipeline::list_capture_sources().map_err(AppError::from)
    }

    /// Not implemented. Needs a one-shot capture per source, which nothing builds yet.
    ///
    /// Reported rather than faked: a shell handed a blank `Thumbnail` would render an empty
    /// grey tile and look broken, where an error lets it fall back to showing titles.
    fn capture_thumbnail(&self, _id: &CaptureSourceId, _max_px: u32) -> Result<Thumbnail> {
        Err(AppError::recoverable(
            ErrorCode::Internal,
            "Source previews are not available yet.",
        )
        .with_hint("Pick a source by its name for now."))
    }

    fn list_cameras(&self) -> Result<Vec<CameraInfo>> {
        crate::camera_pipeline::list_cameras().map_err(AppError::from)
    }

    /// Default endpoints only.
    ///
    /// The capture stack takes "the microphone" and "system audio" as booleans (WASAPI's
    /// default endpoints) and has no per-device selection, so this reports exactly the two
    /// choices that can actually be made rather than an enumeration a shell could not honour.
    /// Named-device selection has no milestone yet.
    fn list_audio_devices(&self) -> Result<AudioDevices> {
        let probe = crate::audio_pipeline::probe();
        let entry = |present: bool, id: &str, name: &str| {
            present.then(|| AudioDevice {
                id: id.to_string(),
                name: name.to_string(),
                is_default: true,
            })
        };
        Ok(AudioDevices {
            inputs: entry(probe.mic, "default", "Default microphone")
                .into_iter()
                .collect(),
            outputs: entry(probe.system, "default", "System audio")
                .into_iter()
                .collect(),
        })
    }

    fn configure(&self, config: SessionConfig) -> Result<()> {
        self.inner.lock().config = config;
        Ok(())
    }

    /// Proves what can be proven cheaply, before the user presses record.
    ///
    /// It resolves FFmpeg and probes for an H.264 encoder, because `h264_mf` is listed in every
    /// Windows build and absent on machines with no Media Foundation encoder — and discovering
    /// that after the record button costs the recording. It does **not** open the camera or the
    /// audio devices: those are opened by `start`, which reports their failures the same way.
    /// So `Ready` means "the encode path works", not "every device is proven".
    fn arm(&self) -> Result<()> {
        let ffmpeg = self.shell.ffmpeg_path()?;
        pipeline::probe_encoder(&ffmpeg).map_err(AppError::from)?;

        let mut inner = self.inner.lock();
        inner.error = None;
        if inner.machine.state() == SessionState::Idle {
            self.transition(&mut inner, SessionState::Ready);
        }
        Ok(())
    }

    fn start(&self) -> Result<SessionId> {
        // Everything fallible happens before the machine moves, so a failed start leaves the
        // session exactly where it was rather than in a Recording state with no worker.
        let id = SessionId(self.next_session.fetch_add(1, Ordering::Relaxed));
        let output = self.output_path(id)?;
        // Built and discarded, on purpose: it is the cheapest way to prove the configuration
        // can *become* a request — FFmpeg resolves, the data directory is writable — while the
        // session is still safely in its previous state. `spawn` builds the one that runs.
        let _validated = {
            let inner = self.inner.lock();
            if inner.machine.state() == SessionState::Recording {
                return Err(AppError::recoverable(
                    ErrorCode::IllegalState,
                    "A recording is already running.",
                ));
            }
            self.build_request(&inner.config, output)?
        };

        let stop = Arc::new(AtomicBool::new(false));
        *self.stop.lock() = Arc::clone(&stop);

        {
            let mut inner = self.inner.lock();
            inner.session_id = Some(id);
            inner.elapsed_ms = 0;
            inner.stats = CaptureStats::default();
            inner.recording = None;
            inner.error = None;
            if inner.machine.state() == SessionState::Idle {
                self.transition(&mut inner, SessionState::Ready);
            }
            self.transition(&mut inner, SessionState::Recording);
        }

        Ok(id)
    }

    /// Signals the worker and returns immediately.
    ///
    /// The recording is not finished when this returns — FFmpeg is still draining, which takes
    /// seconds. `Stopped` arrives as a `StateChanged` event when it genuinely is finished, and
    /// a shell that greys its button out on `stop()` and re-enables it on that event shows the
    /// user the truth rather than a lie that is usually close enough.
    fn stop(&self) -> Result<()> {
        self.stop.lock().store(true, Ordering::Relaxed);
        let mut inner = self.inner.lock();
        if inner.machine.state() == SessionState::Recording {
            self.transition(&mut inner, SessionState::Finalizing);
        }
        Ok(())
    }

    /// Stops, waits for the worker, and deletes the partial file.
    ///
    /// The one method that blocks, because the alternative is deleting a file FFmpeg still has
    /// open. Bounded by [`CANCEL_GRACE`].
    fn cancel(&self) -> Result<()> {
        self.stop.lock().store(true, Ordering::Relaxed);
        let handle = self.worker.lock().take();
        if let Some(handle) = handle {
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                let _ = handle.join();
                let _ = done_tx.send(());
            });
            if done_rx.recv_timeout(CANCEL_GRACE).is_err() {
                tracing::warn!("worker did not stop within {CANCEL_GRACE:?} of cancel");
            }
        }

        let mut inner = self.inner.lock();
        if let Some(recording) = inner.recording.take() {
            let _ = std::fs::remove_file(&recording.path);
        } else if let Some(id) = inner.session_id {
            // The worker never produced a `Recording`, so the path is derived rather than read
            // back — a partial capture is still on disk and is exactly what cancel must remove.
            if let Ok(path) = self.output_path(id) {
                let _ = std::fs::remove_file(path);
            }
        }
        inner.session_id = None;
        inner.elapsed_ms = 0;
        inner.stats = CaptureStats::default();
        self.transition(&mut inner, SessionState::Idle);
        Ok(())
    }

    fn reset(&self) -> Result<()> {
        let mut inner = self.inner.lock();
        inner.session_id = None;
        inner.elapsed_ms = 0;
        inner.stats = CaptureStats::default();
        // Dropped, not deleted: `reset` discards the engine's *reference* to a finished
        // recording, and the file is the user's. `cancel` is the one that deletes.
        inner.recording = None;
        inner.error = None;
        self.transition(&mut inner, SessionState::Idle);
        Ok(())
    }

    fn preview_edit(&self, edit: &EditSpec) -> Result<EditPreview> {
        let inner = self.inner.lock();
        let recording = inner.recording.as_ref().ok_or_else(|| {
            AppError::recoverable(ErrorCode::IllegalState, "There is no recording to edit.")
        })?;
        let start = edit.start_ms.unwrap_or(0);
        let end = edit.end_ms.unwrap_or(recording.duration_ms);
        Ok(EditPreview {
            duration_ms: end.saturating_sub(start),
            passthrough: edit.is_passthrough(),
        })
    }

    /// Not implemented — M7. `vrecnu_encode::render_args` already builds and tests the command
    /// line; what is missing is running it and reporting progress.
    fn apply_edit(&self, _edit: EditSpec) -> Result<RenderId> {
        Err(AppError::recoverable(
            ErrorCode::Internal,
            "Editing is not available yet.",
        ))
    }

    fn save_copy_to(&self, destination: &Path) -> Result<()> {
        let source = {
            let inner = self.inner.lock();
            inner
                .recording
                .as_ref()
                .map(|recording| recording.path.clone())
                .ok_or_else(|| {
                    AppError::recoverable(ErrorCode::IllegalState, "There is no recording to save.")
                })?
        };
        std::fs::copy(&source, destination).map_err(|err| {
            AppError::recoverable(ErrorCode::Internal, "vrecnu could not save a copy.")
                .with_detail(err.to_string())
        })?;
        Ok(())
    }

    /// Starts the loopback publish server and hands back the URL to open.
    ///
    /// The engine does **not** open a browser or a web view: that is `PublishSurface`'s job,
    /// and keeping it there is what lets the same engine drive a Tauri window, a `wry` window
    /// and `vrecnu-cli` printing a URL to stdout.
    ///
    /// The engine also never sees an OAuth token, a Drive request or a byte of the upload. It
    /// serves one file over loopback and waits to be told a link exists (AGENTS.md §5).
    fn begin_publish(&self) -> Result<PublishHandle> {
        let recording = {
            let inner = self.inner.lock();
            inner.recording.clone().ok_or_else(|| {
                AppError::recoverable(ErrorCode::IllegalState, "There is nothing to publish yet.")
                    .with_hint("Record something first.")
            })?
        };

        // One recording, one session, one token. A second `begin_publish` would leave the
        // previous token live against a file nobody is publishing any more.
        let mut slot = self.publish.lock();
        if slot.as_ref().is_some_and(PublishPump::is_running) {
            return Err(AppError::recoverable(
                ErrorCode::IllegalState,
                "A publish is already in progress.",
            ));
        }
        *slot = None;

        let assets = PublishAssets::embedded().map_err(EngineError::Publish)?;

        // Size is re-read from disk rather than taken from `Recording`: an edit may have
        // rewritten the file since the recording finished, and a stale length becomes a
        // truncated upload that Drive accepts without complaint.
        let size_bytes = std::fs::metadata(&recording.path)
            .map(|meta| meta.len())
            .unwrap_or(recording.size_bytes);
        let file_name = recording.path.file_name().map_or_else(
            || "vrecnu-recording.mp4".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );

        let server = PublishServer::start(
            RecordingToServe {
                path: recording.path.clone(),
                file_name,
                mime_type: recording.mime_type.clone(),
                size_bytes,
                duration_ms: Some(recording.duration_ms),
                // A zero duration means the muxer never wrote one, which is exactly the case
                // where Drive's preview player cannot seek. The page says so rather than
                // letting the user discover it after uploading.
                duration_valid: recording.duration_ms > 0,
            },
            assets,
        )
        .map_err(EngineError::Publish)?;

        let handle = PublishHandle {
            url: server.url().to_owned(),
            port: server.port(),
        };

        let stop = Arc::new(AtomicBool::new(false));
        let pump = spawn_publish_pump(
            server,
            Arc::clone(&stop),
            Arc::clone(&self.subscribers),
            Arc::clone(&self.publishing),
            Arc::clone(&self.shell),
        );
        self.publishing.store(true, Ordering::SeqCst);
        *slot = Some(PublishPump {
            stop,
            join: Some(pump),
        });
        drop(slot);

        // Deliberately logs the port and not the URL: the URL carries the capability token.
        tracing::info!(port = handle.port, "publish session opened");
        self.emit(&EngineEvent::PublishOpened {
            url: handle.url.clone(),
            port: handle.port,
        });
        Ok(handle)
    }

    fn abort_publish(&self) -> Result<()> {
        let pump = self.publish.lock().take();
        if let Some(mut pump) = pump {
            pump.stop();
        }
        self.publishing.store(false, Ordering::SeqCst);
        Ok(())
    }

    /// The *out-of-band* path, for a shell that intercepts the publish result itself.
    ///
    /// ADR-006 requires the normal path to be `POST /api/complete` on the loopback server
    /// rather than shell IPC, so `PublishSurface::provides_ipc_result_channel` defaults to
    /// `false` and most shells never call this. It exists so that a shell which genuinely
    /// cannot serve the loopback page is not forced to fake an HTTP request to itself.
    fn submit_publish_result(&self, result: PublishResult) -> Result<()> {
        let _ = self.shell.set_clipboard_text(&result.web_view_link);
        self.emit(&EngineEvent::PublishDone {
            web_view_link: result.web_view_link,
            file_id: result.file_id,
        });
        self.abort_publish()
    }

    fn snapshot(&self) -> AppState {
        self.inner
            .lock()
            .snapshot(self.publishing.load(Ordering::SeqCst))
    }

    fn subscribe(&self) -> Receiver<EngineEvent> {
        let (tx, rx) = unbounded();
        self.subscribers.lock().push(tx);
        rx
    }
}

impl PublishPump {
    /// Whether the pump thread is still alive.
    ///
    /// The thread exits on its own when the page reports a terminal result, so a `Some` in the
    /// slot does not mean a publish is still in progress.
    fn is_running(&self) -> bool {
        self.join.as_ref().is_some_and(|join| !join.is_finished())
    }

    /// Stops the pump and, with it, the loopback server it owns.
    fn stop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

impl Drop for PublishPump {
    fn drop(&mut self) {
        self.stop();
    }
}

/// The MIME type to declare to Drive for an adopted file.
///
/// Only the containers vrecnu itself writes, plus WebM because that is what the Phase 1 PWA
/// falls back to. Anything else is left to Drive to sniff rather than mislabelled.
fn mime_for_extension(path: &Path) -> String {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("mp4" | "m4v") => "video/mp4".to_owned(),
        Some("webm") => "video/webm".to_owned(),
        Some("mkv") => "video/x-matroska".to_owned(),
        _ => "application/octet-stream".to_owned(),
    }
}

/// Turns loopback-server signals into engine events, then releases the port.
///
/// This thread owns `server`, so the registered port is bound for exactly as long as the pump
/// runs. Putting the server anywhere else would create a second thing to remember to close,
/// and forgetting it makes the *next* publish fail with `AllPortsBusy` -- an error that reads
/// like a user problem and is not one.
fn spawn_publish_pump(
    server: PublishServer,
    stop: Arc<AtomicBool>,
    subscribers: Arc<Mutex<Vec<Sender<EngineEvent>>>>,
    publishing: Arc<AtomicBool>,
    shell: Arc<dyn ShellServices>,
) -> JoinHandle<()> {
    let signals = server.signals().clone();
    std::thread::Builder::new()
        .name("vrecnu-publish-pump".into())
        .spawn(move || {
            let emit = |event: EngineEvent| {
                subscribers
                    .lock()
                    .retain(|tx| tx.send(event.clone()).is_ok());
            };

            while !stop.load(Ordering::SeqCst) {
                let signal = match signals.recv_timeout(Duration::from_millis(200)) {
                    Ok(signal) => signal,
                    Err(RecvTimeoutError::Timeout) => continue,
                    // The server loop exited without a terminal signal. Nothing to report.
                    Err(RecvTimeoutError::Disconnected) => break,
                };

                let terminal = match signal {
                    PublishSignal::Progress { bytes_sent, total } => {
                        emit(EngineEvent::PublishProgress { bytes_sent, total });
                        false
                    }
                    PublishSignal::Complete {
                        file_id,
                        web_view_link,
                    } => {
                        // The magic moment: the link is on the clipboard before the user has
                        // finished reading the success message. A clipboard failure must not
                        // turn a successful upload into a failed publish, so it is logged and
                        // the link is still reported.
                        if let Err(err) = shell.set_clipboard_text(&web_view_link) {
                            tracing::warn!(%err, "could not copy the share link");
                        }
                        emit(EngineEvent::PublishDone {
                            web_view_link,
                            file_id,
                        });
                        true
                    }
                    PublishSignal::Failed { message } => {
                        emit(EngineEvent::PublishFailed {
                            error: AppError::from(EngineError::Publish(
                                vrecnu_publish_server::PublishError::PageReported(message),
                            )),
                        });
                        true
                    }
                    PublishSignal::TimedOut { after } => {
                        emit(EngineEvent::PublishFailed {
                            error: AppError::from(EngineError::Publish(
                                vrecnu_publish_server::PublishError::TimedOut(after),
                            )),
                        });
                        true
                    }
                };
                if terminal {
                    break;
                }
            }

            publishing.store(false, Ordering::SeqCst);
            // Explicit, so "the port is released here" is visible at the site rather than
            // being an accident of where the binding happens to end.
            drop(server);
            tracing::debug!("publish pump finished");
        })
        .expect("the OS must be able to spawn the publish pump thread")
}

/// Starts the worker for a session that [`RecorderEngine::start`] has already moved into
/// `Recording`.
///
/// A free function taking `Arc<Session>` rather than a method, because the worker outlives the
/// call and must own its handle on the session — `&self` cannot be moved into a thread.
///
/// Split from `start` for the same reason: `start` is a trait method with `&self`, and the
/// worker needs `Arc<Self>`. A shell calls [`spawn`] instead of `start` directly; see its use
/// in `vrecnu-cli`'s shell and in the Tauri shell's command layer.
pub fn spawn(session: &Arc<Session>) -> Result<SessionId> {
    let id = RecorderEngine::start(session.as_ref())?;
    let request = {
        let inner = session.inner.lock();
        let output = session.output_path(id)?;
        session.build_request(&inner.config, output)?
    };
    let stop = Arc::clone(&session.stop.lock());

    let ctx = WorkerContext {
        session: Arc::clone(session),
        request,
        stop,
        id,
    };
    let handle = std::thread::Builder::new()
        .name("vrecnu-session".into())
        .spawn(move || run_worker(ctx))
        .map_err(|err| {
            AppError::fatal(ErrorCode::Internal, "vrecnu could not start recording.")
                .with_detail(err.to_string())
        })?;
    *session.worker.lock() = Some(handle);
    Ok(id)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A shell that resolves nothing, for the paths that never reach a device.
    struct StubShell {
        ffmpeg: Option<PathBuf>,
    }

    impl ShellServices for StubShell {
        fn set_clipboard_text(&self, _text: &str) -> Result<()> {
            Ok(())
        }
        fn open_external_url(&self, _url: &str) -> Result<()> {
            Ok(())
        }
        fn reveal_in_file_manager(&self, _path: &Path) -> Result<()> {
            Ok(())
        }
        fn notify(&self, _level: vrecnu_shell_api::NotifyLevel, _title: &str, _body: &str) {}
        fn ffmpeg_path(&self) -> Result<PathBuf> {
            self.ffmpeg.clone().ok_or_else(|| {
                AppError::fatal(ErrorCode::EncodeMissingBinary, "no ffmpeg in this test")
            })
        }
        fn data_dir(&self) -> Result<PathBuf> {
            Ok(std::env::temp_dir())
        }
    }

    /// A shell that remembers what was put on the clipboard.
    ///
    /// The share link reaching the clipboard is the product's entire promise, so it is worth a
    /// test double rather than being taken on trust from a `()`-returning stub.
    #[derive(Default)]
    struct RecordingShell {
        clipboard: Mutex<Option<String>>,
    }

    impl ShellServices for RecordingShell {
        fn set_clipboard_text(&self, text: &str) -> Result<()> {
            *self.clipboard.lock() = Some(text.to_owned());
            Ok(())
        }
        fn open_external_url(&self, _url: &str) -> Result<()> {
            Ok(())
        }
        fn reveal_in_file_manager(&self, _path: &Path) -> Result<()> {
            Ok(())
        }
        fn notify(&self, _level: vrecnu_shell_api::NotifyLevel, _title: &str, _body: &str) {}
        fn ffmpeg_path(&self) -> Result<PathBuf> {
            Err(AppError::fatal(
                ErrorCode::EncodeMissingBinary,
                "no ffmpeg in this test",
            ))
        }
        fn data_dir(&self) -> Result<PathBuf> {
            Ok(std::env::temp_dir())
        }
    }

    fn session() -> Arc<Session> {
        Arc::new(Session::new(Arc::new(StubShell { ffmpeg: None })))
    }

    #[test]
    fn a_fresh_session_is_idle_and_has_nothing_to_show() {
        let state = session().snapshot();
        assert_eq!(state.session, SessionState::Idle);
        assert!(state.session_id.is_none());
        assert!(state.recording.is_none());
        assert!(!state.publishing);
    }

    #[test]
    fn a_subscriber_receives_state_changes() {
        let session = session();
        let events = session.subscribe();
        let mut inner = session.inner.lock();
        session.transition(&mut inner, SessionState::Ready);
        drop(inner);
        assert_eq!(
            events.try_recv().ok(),
            Some(EngineEvent::StateChanged {
                from: SessionState::Idle,
                to: SessionState::Ready
            })
        );
    }

    #[test]
    fn a_transition_to_the_state_already_held_emits_nothing() {
        // A shell that re-renders on every event would flicker through identical states.
        let session = session();
        let events = session.subscribe();
        let mut inner = session.inner.lock();
        session.transition(&mut inner, SessionState::Idle);
        drop(inner);
        assert!(events.try_recv().is_err());
    }

    #[test]
    fn a_dropped_subscriber_is_pruned_rather_than_blocking_the_engine() {
        // The reason events are fire-and-forget: a webview that closed must not be able to
        // wedge a recording by not draining its channel.
        let session = session();
        drop(session.subscribe());
        assert_eq!(session.subscribers.lock().len(), 1);
        session.emit(&EngineEvent::Elapsed { ms: 1 });
        assert_eq!(session.subscribers.lock().len(), 0);
    }

    #[test]
    fn arming_without_ffmpeg_fails_and_leaves_the_session_idle() {
        // The point of arming: a machine that cannot encode says so *before* the record button,
        // not after, and does not pretend to be Ready in the meantime.
        let session = session();
        let err = RecorderEngine::arm(session.as_ref()).unwrap_err();
        assert_eq!(err.code, ErrorCode::EncodeMissingBinary);
        assert_eq!(session.snapshot().session, SessionState::Idle);
    }

    #[test]
    fn starting_without_ffmpeg_does_not_leave_a_recording_state_behind() {
        // Everything fallible happens before the machine moves. If it did not, a failed start
        // would leave a shell showing "Recording" with no worker behind it.
        let session = session();
        assert!(RecorderEngine::start(session.as_ref()).is_err());
        assert_eq!(session.snapshot().session, SessionState::Idle);
    }

    #[test]
    fn configure_is_readable_back_through_the_next_request() {
        let session = session();
        let config = SessionConfig {
            fps: 60,
            system_audio: false,
            camera: Some(String::new()),
            ..SessionConfig::default()
        };
        RecorderEngine::configure(session.as_ref(), config.clone()).unwrap();
        let stored = session.inner.lock().config.clone();
        assert_eq!(stored, config);
    }

    #[test]
    fn previewing_an_edit_without_a_recording_is_an_illegal_state_not_a_crash() {
        let session = session();
        let err = RecorderEngine::preview_edit(session.as_ref(), &EditSpec::default()).unwrap_err();
        assert_eq!(err.code, ErrorCode::IllegalState);
    }

    #[test]
    fn a_trim_preview_reports_the_trimmed_duration() {
        let session = session();
        session.inner.lock().recording = Some(Recording {
            session: SessionId(1),
            path: PathBuf::from("x.mp4"),
            duration_ms: 10_000,
            size_bytes: 1,
            mime_type: "video/mp4".into(),
        });
        let edit = EditSpec {
            start_ms: Some(2_000),
            end_ms: Some(7_000),
            ..EditSpec::default()
        };
        let preview = RecorderEngine::preview_edit(session.as_ref(), &edit).unwrap();
        assert_eq!(preview.duration_ms, 5_000);
        assert!(!preview.passthrough);
    }

    #[test]
    fn saving_without_a_recording_is_refused_rather_than_writing_an_empty_file() {
        let session = session();
        let err = RecorderEngine::save_copy_to(session.as_ref(), Path::new("out.mp4")).unwrap_err();
        assert_eq!(err.code, ErrorCode::IllegalState);
    }

    #[test]
    fn reset_drops_the_recording_reference_without_deleting_the_file() {
        // The distinction that matters to a user: reset clears the app, cancel throws work away.
        let file = std::env::temp_dir().join("vrecnu-reset-test.mp4");
        std::fs::write(&file, b"x").unwrap();
        let session = session();
        session.inner.lock().recording = Some(Recording {
            session: SessionId(1),
            path: file.clone(),
            duration_ms: 1,
            size_bytes: 1,
            mime_type: "video/mp4".into(),
        });
        RecorderEngine::reset(session.as_ref()).unwrap();
        assert!(session.snapshot().recording.is_none());
        assert!(file.exists(), "reset must not delete the user's recording");
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn unimplemented_milestones_report_themselves_rather_than_faking_success() {
        // A shell that cannot distinguish "no cameras" from "not written yet" ships the wrong
        // UI for it. Each of these names its milestone in the message.
        let session = session();
        assert!(RecorderEngine::apply_edit(session.as_ref(), EditSpec::default()).is_err());
        assert!(RecorderEngine::capture_thumbnail(
            session.as_ref(),
            &CaptureSourceId("monitor:1".into()),
            64
        )
        .is_err());
    }

    #[test]
    fn publishing_with_no_recording_is_an_illegal_state_not_a_bound_port() {
        // Binding first would burn a registered port to discover something checkable for free,
        // and `AllPortsBusy` is a far more confusing error than "record something first".
        let session = session();
        let err = RecorderEngine::begin_publish(session.as_ref()).unwrap_err();
        assert_eq!(err.code, ErrorCode::IllegalState);
        assert!(!err.session_lost, "there was no recording to lose");
        assert!(!RecorderEngine::snapshot(session.as_ref()).publishing);
    }

    #[test]
    fn aborting_a_publish_that_never_started_is_not_an_error() {
        // A shell tears down its publish panel on close without knowing whether one was open.
        let session = session();
        assert!(RecorderEngine::abort_publish(session.as_ref()).is_ok());
        assert!(RecorderEngine::abort_publish(session.as_ref()).is_ok());
    }

    #[test]
    fn an_out_of_band_result_copies_the_link_and_announces_it() {
        // The ADR-006 escape hatch. Even here the link must reach the clipboard, because that
        // is the product's whole promise and a shell should not have to remember to do it.
        let shell = Arc::new(RecordingShell::default());
        let session = Session::new(shell.clone());
        let events = RecorderEngine::subscribe(&session);

        RecorderEngine::submit_publish_result(
            &session,
            PublishResult {
                file_id: "abc123".into(),
                web_view_link: "https://drive.google.com/file/d/abc123/view".into(),
            },
        )
        .unwrap();

        assert_eq!(
            shell.clipboard.lock().as_deref(),
            Some("https://drive.google.com/file/d/abc123/view")
        );
        assert!(matches!(
            events.try_recv().unwrap(),
            EngineEvent::PublishDone { file_id, .. } if file_id == "abc123"
        ));
        assert!(!RecorderEngine::snapshot(&session).publishing);
    }
}
