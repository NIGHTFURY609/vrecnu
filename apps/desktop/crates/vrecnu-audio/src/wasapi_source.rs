//! WASAPI-backed [`AudioSource`] implementations: microphone and system-audio loopback.
//!
//! Loopback capture is a Windows quirk, not a separate API: `AUDCLNT_STREAMFLAGS_LOOPBACK` is
//! only valid on a **render** endpoint, so "capturing system audio" means opening the default
//! *output* device and initializing it for capture anyway. The `wasapi` crate's convention
//! (confirmed against its own `examples/record.rs`) is that passing `Direction::Render`
//! straight through to both device selection and `initialize_client` is what turns that render
//! device into a loopback capture source — there is no separate loopback flag to set by hand.
//!
//! All the actual WASAPI/COM work happens on one dedicated thread per source, spawned by
//! `start()`. `start()` blocks until that thread confirms the device opened (or reports why it
//! didn't) before returning, mirroring `WgcBackend::start`'s synchronous error propagation.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;

use wasapi::{DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat};

use crate::resample::to_48k_stereo_s16;
use crate::{AudioError, AudioSource, PcmSink, SampleKind, SourceFormat};

/// How long to wait for a WASAPI event before checking the stop flag again.
///
/// Short enough that `stop()` reacts promptly; long enough that the loop isn't spinning between
/// packets on a device with a longer natural period.
const EVENT_TIMEOUT_MS: u32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Mic,
    Loopback,
}

impl Kind {
    const fn direction(self) -> Direction {
        match self {
            Self::Mic => Direction::Capture,
            Self::Loopback => Direction::Render,
        }
    }

    fn no_device_error(self) -> AudioError {
        match self {
            Self::Mic => AudioError::NoInputDevice,
            Self::Loopback => AudioError::NoDefaultOutput,
        }
    }

    fn unsupported_error(self, format: &SourceFormat) -> AudioError {
        match self {
            Self::Loopback => AudioError::LoopbackUnsupported,
            Self::Mic => AudioError::UnsupportedConfig {
                device: "default input device".into(),
                rate: format.sample_rate_hz,
                channels: format.channels,
            },
        }
    }
}

/// Shared implementation behind both public source types.
struct WasapiCapture {
    kind: Kind,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl WasapiCapture {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            // Replaced with a fresh `Arc` in `start()`; this one is never read, but the field
            // still needs a value between construction and the first `start()` call.
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }
}

impl AudioSource for WasapiCapture {
    fn start(&mut self, sink: Box<dyn PcmSink>) -> Result<(), AudioError> {
        if self.thread.is_some() {
            return Err(AudioError::Backend(
                "audio capture is already running".into(),
            ));
        }

        let stop = Arc::new(AtomicBool::new(false));
        let stop_for_thread = Arc::clone(&stop);
        let kind = self.kind;
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), AudioError>>();

        let handle =
            std::thread::spawn(move || capture_loop(kind, sink, &stop_for_thread, &ready_tx));

        // Block until the device is confirmed open, or confirmed failed — the same synchronous
        // error contract `WgcBackend::start` gives its caller, rather than discovering a bad
        // device only once the first `on_pcm` never arrives.
        match ready_rx.recv() {
            Ok(Ok(())) => {
                self.stop = stop;
                self.thread = Some(handle);
                Ok(())
            }
            Ok(Err(err)) => {
                let _ = handle.join();
                Err(err)
            }
            Err(_) => {
                let _ = handle.join();
                Err(AudioError::Backend(
                    "audio capture thread exited before reporting readiness".into(),
                ))
            }
        }
    }

    fn stop(&mut self) -> Result<(), AudioError> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.thread.take() {
            let _ = handle.join();
        }
        Ok(())
    }
}

/// Captures the default microphone.
pub struct WasapiMicSource(WasapiCapture);

impl WasapiMicSource {
    /// A source that is not yet capturing.
    #[must_use]
    pub fn new() -> Self {
        Self(WasapiCapture::new(Kind::Mic))
    }
}

impl Default for WasapiMicSource {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioSource for WasapiMicSource {
    fn start(&mut self, sink: Box<dyn PcmSink>) -> Result<(), AudioError> {
        self.0.start(sink)
    }
    fn stop(&mut self) -> Result<(), AudioError> {
        self.0.stop()
    }
}

/// Captures system audio via WASAPI loopback on the default output device.
pub struct WasapiLoopbackSource(WasapiCapture);

impl WasapiLoopbackSource {
    /// A source that is not yet capturing.
    #[must_use]
    pub fn new() -> Self {
        Self(WasapiCapture::new(Kind::Loopback))
    }
}

impl Default for WasapiLoopbackSource {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioSource for WasapiLoopbackSource {
    fn start(&mut self, sink: Box<dyn PcmSink>) -> Result<(), AudioError> {
        self.0.start(sink)
    }
    fn stop(&mut self) -> Result<(), AudioError> {
        self.0.stop()
    }
}

/// Runs entirely on its own thread: opens the device, signals readiness on `ready`, then pumps
/// packets into `sink` until `stop` is set or the device stops delivering events.
fn capture_loop(
    kind: Kind,
    mut sink: Box<dyn PcmSink>,
    stop: &AtomicBool,
    ready: &mpsc::Sender<Result<(), AudioError>>,
) {
    if let Err(err) = wasapi::initialize_mta().ok() {
        let _ = ready.send(Err(AudioError::Backend(Box::new(err))));
        return;
    }

    let opened = open_device(kind);
    let (audio_client, capture_client, event_handle, source_format) = match opened {
        Ok(parts) => {
            let _ = ready.send(Ok(()));
            parts
        }
        Err(err) => {
            let _ = ready.send(Err(err));
            wasapi::deinitialize();
            return;
        }
    };

    let mut queue: VecDeque<u8> = VecDeque::new();
    while !stop.load(Ordering::Relaxed) {
        if event_handle.wait_for_event(EVENT_TIMEOUT_MS).is_err() {
            // A timeout is expected and routine — WASAPI has nothing new, not a failure. Any
            // other error means the device is gone; either way, loop back and let the stop flag
            // (checked above) be the one thing that actually ends capture.
            continue;
        }

        loop {
            let Ok(Some(frames)) = capture_client.get_next_packet_size() else {
                break;
            };
            if frames == 0 {
                break;
            }
            let Ok(_info) = capture_client.read_from_device_to_deque(&mut queue) else {
                break;
            };
            if queue.is_empty() {
                continue;
            }
            let raw: Vec<u8> = queue.drain(..).collect();
            let pcm = to_48k_stereo_s16(&raw, source_format);
            if !pcm.is_empty() {
                sink.on_pcm(&pcm, 0);
            }
        }
    }

    let _ = audio_client.stop_stream();
    wasapi::deinitialize();
}

type OpenedDevice = (
    wasapi::AudioClient,
    wasapi::AudioCaptureClient,
    wasapi::Handle,
    SourceFormat,
);

fn open_device(kind: Kind) -> Result<OpenedDevice, AudioError> {
    let direction = kind.direction();

    let enumerator = DeviceEnumerator::new().map_err(|err| AudioError::Backend(Box::new(err)))?;
    let device = enumerator
        .get_default_device(&direction)
        .map_err(|_| kind.no_device_error())?;
    let mut audio_client = device
        .get_iaudioclient()
        .map_err(|err| AudioError::Backend(Box::new(err)))?;
    let mix_format = audio_client
        .get_mixformat()
        .map_err(|err| AudioError::Backend(Box::new(err)))?;
    let source_format = source_format_from(&mix_format)?;

    let (_default_period, min_period) = audio_client
        .get_device_period()
        .map_err(|err| AudioError::Backend(Box::new(err)))?;
    // No autoconvert: we ask for the device's own native mix format (guaranteed acceptable) and
    // do our own resampling in `resample.rs`, deliberately, so the conversion is one piece of
    // testable code rather than an OS behaviour we'd otherwise have to trust blindly.
    let mode = StreamMode::EventsShared {
        autoconvert: false,
        buffer_duration_hns: min_period,
    };
    audio_client
        .initialize_client(&mix_format, &direction, &mode)
        .map_err(|_| kind.unsupported_error(&source_format))?;

    let event_handle = audio_client
        .set_get_eventhandle()
        .map_err(|err| AudioError::Backend(Box::new(err)))?;
    let capture_client = audio_client
        .get_audiocaptureclient()
        .map_err(|err| AudioError::Backend(Box::new(err)))?;
    audio_client
        .start_stream()
        .map_err(|err| AudioError::Backend(Box::new(err)))?;

    Ok((audio_client, capture_client, event_handle, source_format))
}

/// Only 16-bit int and 32-bit float mix formats are handled — by far the common case for a
/// WASAPI shared-mode default device. Anything else is reported rather than silently
/// misinterpreted at the wrong bit width.
fn source_format_from(format: &WaveFormat) -> Result<SourceFormat, AudioError> {
    let rate = format.get_samplespersec();
    let channels = format.get_nchannels();
    let bits = format.get_bitspersample();

    let sample_kind = match format
        .get_subformat()
        .map_err(|err| AudioError::Backend(Box::new(err)))?
    {
        SampleType::Float if bits == 32 => SampleKind::F32,
        SampleType::Int if bits == 16 => SampleKind::S16,
        _ => {
            return Err(AudioError::UnsupportedConfig {
                device: "default audio device".into(),
                rate,
                channels,
            })
        }
    };

    Ok(SourceFormat {
        sample_rate_hz: rate,
        channels,
        sample_kind,
    })
}
