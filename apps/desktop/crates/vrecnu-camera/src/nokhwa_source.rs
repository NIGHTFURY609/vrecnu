//! The Media Foundation backend, via `nokhwa`.
//!
//! Thin on purpose. Everything that involves a *decision* — which format to open — lives in
//! [`crate::format`] where it can be tested without hardware; what is left here is opening the
//! device, decoding, and fanning the result out.
//!
//! **MJPEG is declined at negotiation time.** `nokhwa`'s MJPEG decoder is behind its `decoding`
//! feature, which pulls in `mozjpeg-sys` and with it a NASM build prerequisite for everyone who
//! clones this repository — a real cost on an open-source project whose contributors are not
//! all set up for native builds. Every Media Foundation camera we have seen also advertises
//! NV12 or YUY2, so the format list handed to `nokhwa` simply excludes MJPEG and negotiation
//! picks one of those. A device that offers *only* MJPEG fails with
//! [`CameraError::FormatNegotiationFailed`], and the fix is one cargo feature: build with
//! `--features mjpeg` (see this crate's `Cargo.toml`). Choosing which formats to request,
//! rather than requesting all of them and discovering at decode time that one cannot be read,
//! is what keeps that failure at *open* time — before the user has pressed record.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::JoinHandle;
use std::time::Instant;

use nokhwa::pixel_format::RgbAFormat;
use nokhwa::utils::{
    ApiBackend, CameraFormat, CameraIndex, FrameFormat, RequestedFormat, RequestedFormatType,
};
use nokhwa::Camera;
use parking_lot::Mutex;
use vrecnu_types::{CameraInfo, Size};

use crate::format::{select_format, DeviceFormat};
use crate::{CameraConsumer, CameraError, CameraFrame, CameraSource};

/// Formats this build can decode.
///
/// MJPEG is absent unless the `mjpeg` feature is on — see the module doc comment. The order is
/// not a preference; `nokhwa` treats this as a set and [`select_format`] does the choosing.
const DECODABLE: &[FrameFormat] = &[
    FrameFormat::NV12,
    FrameFormat::YUYV,
    FrameFormat::RAWRGB,
    FrameFormat::RAWBGR,
    #[cfg(feature = "mjpeg")]
    FrameFormat::MJPEG,
];

/// How many consecutive frame reads may fail before the capture thread gives up.
///
/// Not one: Media Foundation returns a transient error on the first read or two while the
/// device warms up, and treating that as a dead camera closes a device that was about to work.
/// Not unbounded either — a camera unplugged mid-recording never recovers, and spinning on it
/// burns a core for the rest of the session.
const MAX_CONSECUTIVE_ERRORS: u32 = 30;

/// Shared list of consumers, so [`NokhwaCameraSource::subscribe`] can add one while the capture
/// thread is running.
type Consumers = Arc<Mutex<Vec<Box<dyn CameraConsumer>>>>;

/// Opens the webcam through Media Foundation and broadcasts its frames.
///
/// One device open per instance, and one instance per recording — see the crate doc comment for
/// why opening twice is disqualifying rather than merely wasteful.
pub struct NokhwaCameraSource {
    consumers: Consumers,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl Default for NokhwaCameraSource {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for NokhwaCameraSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NokhwaCameraSource")
            .field("running", &self.thread.is_some())
            .field("consumers", &self.consumers.lock().len())
            .finish()
    }
}

impl NokhwaCameraSource {
    /// A source that has not opened anything yet.
    #[must_use]
    pub fn new() -> Self {
        Self {
            consumers: Arc::new(Mutex::new(Vec::new())),
            stop: Arc::new(AtomicBool::new(false)),
            thread: None,
        }
    }
}

/// Enumerates cameras without opening any of them — cheap enough for a capability report.
///
/// # Errors
/// [`CameraError::Backend`] if Media Foundation itself will not answer.
pub fn enumerate_cameras() -> Result<Vec<CameraInfo>, CameraError> {
    let devices = nokhwa::query(ApiBackend::MediaFoundation).map_err(backend_error)?;
    Ok(devices
        .into_iter()
        .map(|device| CameraInfo {
            id: device.index().to_string(),
            name: device.human_name(),
        })
        .collect())
}

impl CameraSource for NokhwaCameraSource {
    fn enumerate(&self) -> Result<Vec<CameraInfo>, CameraError> {
        enumerate_cameras()
    }

    /// Opens the device **on the capture thread**, and blocks until it is confirmed open.
    ///
    /// Not an implementation detail. `nokhwa::Camera` is deliberately not `Send`, because a
    /// Media Foundation source object has thread affinity: it must be created, read and
    /// released on one thread, and touching it from another is how you get a hang rather than
    /// an error. So the thread opens its own device and reports back over a channel — which
    /// also keeps the property that matters to the caller: a bad or busy camera surfaces
    /// *here*, before FFmpeg is spawned, the same discipline `audio_pipeline::start` applies.
    fn start(&mut self, id: &str, min_size: Size) -> Result<(), CameraError> {
        if self.thread.is_some() {
            // Starting twice would open the device twice, which is the exact failure this
            // crate exists to prevent.
            return Ok(());
        }

        let id = id.to_string();
        let consumers = Arc::clone(&self.consumers);
        let stop = Arc::clone(&self.stop);
        stop.store(false, Ordering::Relaxed);
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), CameraError>>();

        let thread = std::thread::Builder::new()
            .name("vrecnu-camera".into())
            .spawn(move || {
                let camera = match open_device(&id, min_size) {
                    Ok(camera) => {
                        let _ = ready_tx.send(Ok(()));
                        camera
                    }
                    Err(err) => {
                        let _ = ready_tx.send(Err(err));
                        return;
                    }
                };
                capture_loop(camera, &consumers, &stop);
            })
            .map_err(|err| CameraError::Backend(Box::new(err)))?;

        match ready_rx.recv() {
            Ok(Ok(())) => {
                self.thread = Some(thread);
                Ok(())
            }
            Ok(Err(err)) => {
                let _ = thread.join();
                Err(err)
            }
            Err(_) => {
                let _ = thread.join();
                Err(CameraError::Backend(Box::new(CapturedBackendError(
                    "the camera thread exited before the device was open".into(),
                ))))
            }
        }
    }

    fn subscribe(&mut self, consumer: Box<dyn CameraConsumer>) {
        self.consumers.lock().push(consumer);
    }

    fn stop(&mut self) -> Result<(), CameraError> {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            // The loop checks the flag between frames, so this waits at most one frame
            // interval. `Camera::stop_stream` happens inside the loop, on the thread that owns
            // the device — Media Foundation objects have thread affinity and closing one from
            // another thread is how you get a hang instead of a clean release.
            let _ = thread.join();
        }
        self.consumers.lock().clear();
        Ok(())
    }
}

impl Drop for NokhwaCameraSource {
    /// Releasing the device matters more than usual: a webcam left open keeps its activity
    /// light on, which users reasonably read as "it is still recording me".
    fn drop(&mut self) {
        let _ = CameraSource::stop(self);
    }
}

/// Resolves, opens and negotiates the device. Runs on the capture thread — see `start`.
fn open_device(id: &str, min_size: Size) -> Result<Camera, CameraError> {
    let index = resolve_index(id)?;
    let name = device_name(&index);

    // Opened with a format request restricted to what this build can decode, so a device that
    // only speaks MJPEG fails here rather than at the first frame.
    let request =
        RequestedFormat::with_formats(RequestedFormatType::AbsoluteHighestResolution, DECODABLE);
    let mut camera = Camera::new(index, request).map_err(|err| open_error(&name, &err))?;

    let offered = camera
        .compatible_camera_formats()
        .map_err(|err| open_error(&name, &err))?;
    let reduced: Vec<DeviceFormat> = offered
        .iter()
        .map(|format| DeviceFormat {
            width: format.resolution().width(),
            height: format.resolution().height(),
            fps: format.frame_rate(),
        })
        .collect();

    let chosen = select_format(&reduced, min_size).ok_or(CameraError::FormatNegotiationFailed {
        name: name.clone(),
        width: min_size.width,
        height: min_size.height,
    })?;
    let chosen = offered[chosen];

    // `Exact`, not a size hint: the choice has already been made against the device's own
    // reported list, and letting nokhwa re-derive it from a fuzzy request could land somewhere
    // else entirely.
    camera
        .set_camera_requset(RequestedFormat::with_formats(
            RequestedFormatType::Exact(chosen),
            DECODABLE,
        ))
        .map_err(|err| open_error(&name, &err))?;

    camera
        .open_stream()
        .map_err(|err| open_error(&name, &err))?;

    tracing::info!(camera = %name, format = %format_label(chosen), "camera opened");
    Ok(camera)
}

/// Decodes frames until stopped, handing each to every consumer.
fn capture_loop(mut camera: Camera, consumers: &Consumers, stop: &AtomicBool) {
    let resolution = camera.resolution();
    let mut frame = CameraFrame {
        width: resolution.width(),
        height: resolution.height(),
        // Allocated once, here, and reused for every frame — the same rule the capture frame
        // pool follows. A per-frame allocation would be thirty a second for the whole session.
        bgra: vec![0u8; (resolution.width() as usize) * (resolution.height() as usize) * 4],
        timestamp_ns: 0,
    };
    let started = Instant::now();
    let mut consecutive_errors = 0u32;

    while !stop.load(Ordering::Relaxed) {
        let buffer = match camera.frame() {
            Ok(buffer) => {
                consecutive_errors = 0;
                buffer
            }
            Err(err) => {
                consecutive_errors += 1;
                if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                    tracing::warn!(
                        %err,
                        "camera stopped delivering frames; closing the device"
                    );
                    break;
                }
                continue;
            }
        };

        // The device may renegotiate under us (some drivers do this on a resolution change in
        // another application). Resizing rather than trusting the opening resolution is what
        // stops a short buffer from being decoded into a half-filled frame.
        let size = buffer.resolution();
        if size.width() != frame.width || size.height() != frame.height {
            frame.width = size.width();
            frame.height = size.height();
            frame
                .bgra
                .resize((size.width() as usize) * (size.height() as usize) * 4, 0);
        }

        if let Err(err) = buffer.decode_image_to_buffer::<RgbAFormat>(&mut frame.bgra) {
            consecutive_errors += 1;
            if consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                tracing::warn!(%err, "camera frames stopped decoding; closing the device");
                break;
            }
            continue;
        }
        // `nokhwa` decodes to RGBA; everything downstream of here — the bubble renderer, the
        // FFmpeg overlay pipe, the capture pipeline — is BGRA. Swapping in place beats decoding
        // into a second buffer and copying.
        rgba_to_bgra_in_place(&mut frame.bgra);

        frame.timestamp_ns = u64::try_from(started.elapsed().as_nanos()).unwrap_or(u64::MAX);

        // Held across the callbacks so a consumer cannot be dropped mid-call. Consumers are
        // expected to be cheap — the overlay writer stores the frame and returns; it does not
        // do its own I/O here.
        for consumer in consumers.lock().iter_mut() {
            consumer.on_camera_frame(&frame);
        }
    }

    let _ = camera.stop_stream();
}

/// Swaps the red and blue channels of a packed 4-byte-per-pixel buffer.
fn rgba_to_bgra_in_place(buffer: &mut [u8]) {
    for pixel in buffer.chunks_exact_mut(4) {
        pixel.swap(0, 2);
    }
}

/// Turns the id a shell passes back into the index `nokhwa` wants.
fn resolve_index(id: &str) -> Result<CameraIndex, CameraError> {
    if id.is_empty() {
        // "the default camera" — the first one enumerated, which is what Windows itself
        // presents as the default in its own camera settings.
        let devices = nokhwa::query(ApiBackend::MediaFoundation).map_err(backend_error)?;
        let first = devices.first().ok_or(CameraError::NoCamera)?;
        return Ok(first.index().clone());
    }
    Ok(match id.parse::<u32>() {
        Ok(index) => CameraIndex::Index(index),
        Err(_) => CameraIndex::String(id.to_string()),
    })
}

/// Best-effort human name, for error messages only. Falls back to the raw index rather than
/// failing: an error about a camera is not worth turning into a different error about naming it.
fn device_name(index: &CameraIndex) -> String {
    nokhwa::query(ApiBackend::MediaFoundation)
        .ok()
        .and_then(|devices| {
            devices
                .into_iter()
                .find(|device| device.index() == index)
                .map(|device| device.human_name())
        })
        .unwrap_or_else(|| index.to_string())
}

fn format_label(format: CameraFormat) -> String {
    format!(
        "{}x{} {} @{}fps",
        format.resolution().width(),
        format.resolution().height(),
        format.format(),
        format.frame_rate()
    )
}

fn backend_error(err: nokhwa::NokhwaError) -> CameraError {
    CameraError::Backend(Box::new(err))
}

/// Classifies an open failure into something a user can act on.
///
/// `nokhwa` reports all three of these as `OpenDeviceError`/`InitializeError` with the OS
/// message in a string, so the string is what there is to go on. Getting this wrong is not
/// harmless: "camera is in use by another application" and "camera access was denied" have
/// completely different fixes, and a generic backend error tells the user neither.
fn open_error(name: &str, err: &nokhwa::NokhwaError) -> CameraError {
    let text = err.to_string().to_ascii_lowercase();
    if text.contains("access is denied")
        || text.contains("accessdenied")
        || text.contains("0x80070005")
    {
        return CameraError::PermissionDenied;
    }
    if text.contains("in use")
        || text.contains("busy")
        || text.contains("0xc00d3704")
        || text.contains("hardware in use")
    {
        return CameraError::DeviceBusy {
            name: name.to_string(),
        };
    }
    CameraError::Backend(Box::new(CapturedBackendError(err.to_string())))
}

/// Carries a backend message that has already been formatted, so [`CameraError::Backend`] can
/// hold it without borrowing the original error.
#[derive(Debug)]
struct CapturedBackendError(String);

impl std::fmt::Display for CapturedBackendError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for CapturedBackendError {}

/// Whether at least one camera exists, for a capability report like `doctor`.
///
/// Cheap: enumerates endpoints, opens nothing.
#[must_use]
pub fn camera_available() -> bool {
    enumerate_cameras().is_ok_and(|cameras| !cameras.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rgba_becomes_bgra_by_swapping_red_and_blue() {
        // The one-line bug this guards: leaving the channels alone renders a blue face, which
        // looks like a lighting problem rather than a byte-order problem.
        let mut buffer = vec![10, 20, 30, 255, 40, 50, 60, 255];
        rgba_to_bgra_in_place(&mut buffer);
        assert_eq!(buffer, vec![30, 20, 10, 255, 60, 50, 40, 255]);
    }

    #[test]
    fn the_swap_leaves_alpha_untouched() {
        let mut buffer = vec![1, 2, 3, 128];
        rgba_to_bgra_in_place(&mut buffer);
        assert_eq!(buffer[3], 128);
    }

    #[test]
    fn a_numeric_id_resolves_to_an_index_without_touching_the_device() {
        assert_eq!(resolve_index("2").unwrap(), CameraIndex::Index(2));
    }

    #[test]
    fn a_non_numeric_id_is_kept_as_a_device_path() {
        // Media Foundation symbolic links are the stable id across reboots; a positional index
        // is not, and silently reinterpreting one as the other opens the wrong camera.
        let id = r"\\?\usb#vid_046d";
        assert_eq!(
            resolve_index(id).unwrap(),
            CameraIndex::String(id.to_string())
        );
    }

    #[test]
    fn an_access_denied_message_is_reported_as_a_permission_problem() {
        // Windows camera privacy settings produce this, and "denied" and "busy" have entirely
        // different fixes — a generic backend error tells the user neither.
        let err = nokhwa::NokhwaError::OpenDeviceError(
            "0".into(),
            "Access is denied. (0x80070005)".into(),
        );
        assert!(matches!(
            open_error("Webcam", &err),
            CameraError::PermissionDenied
        ));
    }

    #[test]
    fn a_hardware_in_use_message_is_reported_as_busy_and_names_the_device() {
        let err = nokhwa::NokhwaError::OpenDeviceError(
            "0".into(),
            "The hardware in use error 0xC00D3704".into(),
        );
        match open_error("Logi Webcam", &err) {
            CameraError::DeviceBusy { name } => assert_eq!(name, "Logi Webcam"),
            other => panic!("expected DeviceBusy, got {other:?}"),
        }
    }

    #[test]
    fn an_unrecognised_message_stays_a_backend_error_rather_than_being_guessed_at() {
        let err = nokhwa::NokhwaError::OpenDeviceError("0".into(), "something unusual".into());
        assert!(matches!(
            open_error("Webcam", &err),
            CameraError::Backend(_)
        ));
    }

    #[test]
    fn mjpeg_is_not_requested_unless_the_feature_asks_for_it() {
        // The build would otherwise negotiate a format it cannot decode, and the failure would
        // land at the first frame — after the user pressed record — instead of at open.
        assert_eq!(
            DECODABLE.contains(&FrameFormat::MJPEG),
            cfg!(feature = "mjpeg")
        );
    }

    #[test]
    fn a_fresh_source_has_no_thread_and_no_consumers() {
        let source = NokhwaCameraSource::new();
        assert!(source.thread.is_none());
        assert_eq!(source.consumers.lock().len(), 0);
    }

    #[test]
    fn subscribing_before_start_is_allowed() {
        // The engine subscribes the overlay writer and the bubble before opening the device, so
        // no frame can slip past between `start` and the first `subscribe`.
        struct Noop;
        impl CameraConsumer for Noop {
            fn on_camera_frame(&mut self, _frame: &CameraFrame) {}
        }
        let mut source = NokhwaCameraSource::new();
        source.subscribe(Box::new(Noop));
        assert_eq!(source.consumers.lock().len(), 1);
    }
}
