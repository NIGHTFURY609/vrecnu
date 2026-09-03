use std::path::PathBuf;

use vrecnu_audio::AudioError;
use vrecnu_camera::CameraError;
use vrecnu_capture::CaptureError;
use vrecnu_encode::EncodeError;
use vrecnu_publish_server::PublishError;
use vrecnu_shell_api::AppError;
use vrecnu_types::{ErrorCode, SessionState};

/// Every failure the engine can produce, aggregated from the leaf crates.
///
/// Rich and non-serializable on purpose. The flat, serializable `AppError` is derived from it
/// at the boundary — see the `From` impl below, which is the *only* place user-facing error
/// text is written.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EngineError {
    /// Screen capture failed.
    #[error(transparent)]
    Capture(#[from] CaptureError),
    /// Audio capture failed.
    #[error(transparent)]
    Audio(#[from] AudioError),
    /// Camera capture failed.
    #[error(transparent)]
    Camera(#[from] CameraError),
    /// The FFmpeg sidecar failed.
    #[error(transparent)]
    Encode(#[from] EncodeError),
    /// The publish server failed.
    #[error(transparent)]
    Publish(#[from] PublishError),
    /// The named pipe carrying mixed audio to FFmpeg failed.
    #[error(transparent)]
    Pipe(#[from] vrecnu_namedpipe::PipeError),
    /// The floating camera bubble window could not be created.
    #[error(transparent)]
    Bubble(#[from] vrecnu_bubble_win32::BubbleError),
    /// The requested action is not legal in the current state.
    #[error("cannot {action} while {state:?}")]
    IllegalTransition {
        /// State the machine was in.
        state: SessionState,
        /// Action that was attempted.
        action: &'static str,
    },
    /// Not enough room to record.
    #[error("need {needed_mb} MB but only {free_mb} MB are free at {path}")]
    InsufficientDisk {
        /// Megabytes required.
        needed_mb: u64,
        /// Megabytes available.
        free_mb: u64,
        /// Where the check was made.
        path: PathBuf,
    },
    /// An operation needed a session and there was none.
    #[error("there is no active recording session")]
    NoSession,
    /// Anything else.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl From<EngineError> for AppError {
    /// The one place user-facing error text exists.
    ///
    /// `session_lost` is set independently of `recoverable`, because "the encoder died and
    /// your recording is gone" and "port 47821 is busy" are both things the user can act on
    /// and must not be rendered with the same weight.
    fn from(err: EngineError) -> Self {
        let detail = err.to_string();
        let base = match &err {
            EngineError::Capture(CaptureError::Unavailable) => AppError::recoverable(
                ErrorCode::CaptureUnavailable,
                "vrecnu can't record on this version of Windows.",
            )
            .with_hint("Screen recording needs Windows 10 version 1903 or newer."),

            EngineError::Capture(CaptureError::PermissionDenied) => AppError::recoverable(
                ErrorCode::CapturePermission,
                "Screen recording permission was denied.",
            )
            .with_hint("Allow screen recording, then start again."),

            EngineError::Capture(CaptureError::SourceGone(_)) => AppError::recoverable(
                ErrorCode::CaptureSourceGone,
                "The window or screen being recorded has closed.",
            )
            .with_hint("Pick another source and start again."),

            // "Nothing arrived at all" is a different failure from "the screen was static":
            // the pacer handles the second, and reporting it as a capture failure would be a
            // lie. This arm only fires when the session never produced a single frame.
            EngineError::Capture(CaptureError::NoFrames { .. }) => AppError::recoverable(
                ErrorCode::CaptureSourceGone,
                "vrecnu couldn't get any picture from what you picked.",
            )
            .with_hint("Make sure the window isn't minimised, then start again."),

            EngineError::Capture(CaptureError::SizeChanged { .. }) => AppError::recoverable(
                ErrorCode::CaptureSourceGone,
                "The screen resolution changed while recording, so recording stopped.",
            )
            .with_hint("Start again without changing display settings mid-recording."),

            EngineError::Capture(CaptureError::Backpressure { .. }) => AppError::recoverable(
                ErrorCode::CaptureBackpressure,
                "Your computer couldn't keep up and some frames were dropped.",
            )
            .with_hint("Try a lower frame rate, or close some other applications."),

            EngineError::Audio(AudioError::LoopbackUnsupported | AudioError::NoDefaultOutput) => {
                AppError::recoverable(
                    ErrorCode::AudioNoLoopback,
                    "System audio couldn't be captured on this machine.",
                )
                .with_hint("You can still record with just your microphone.")
            }

            EngineError::Audio(AudioError::DeviceDisconnected { .. }) => AppError::recoverable(
                ErrorCode::AudioDeviceLost,
                "An audio device was disconnected during recording.",
            ),

            EngineError::Camera(CameraError::DeviceBusy { .. }) => AppError::recoverable(
                ErrorCode::CameraBusy,
                "Your camera is being used by another application.",
            )
            .with_hint("Close the other app using the camera, then try again."),

            EngineError::Camera(CameraError::NoCamera) => {
                AppError::recoverable(ErrorCode::CameraNone, "No camera was found.")
            }

            EngineError::Camera(CameraError::PermissionDenied) => AppError::recoverable(
                ErrorCode::CameraPermission,
                "Camera access was denied.",
            )
            .with_hint("Allow camera access in Windows privacy settings, then try again."),

            // The bubble is the *window*, not the camera: the recording's burned-in overlay
            // does not depend on it, so this is deliberately not reported as a camera failure.
            // It is still fatal to `start`, because a user who asked for a bubble and silently
            // did not get one has no way to tell that from a broken camera.
            EngineError::Bubble(_) => AppError::recoverable(
                ErrorCode::Internal,
                "The floating camera window could not be opened.",
            )
            .with_hint("Recording without the floating window still includes your camera."),

            // Residual arms for the three subsystems above. Required, not optional: their
            // error enums are `#[non_exhaustive]`, so this crate can never match them
            // exhaustively and the compiler will keep demanding a fallback as they grow.
            // Each keeps its subsystem's voice; the precise cause survives in `detail`.
            EngineError::Capture(_) => {
                AppError::recoverable(ErrorCode::Internal, "Screen recording failed.")
            }
            EngineError::Audio(_) => {
                AppError::recoverable(ErrorCode::Internal, "Audio couldn't be recorded.")
                    .with_hint("You can record without audio, or pick a different device.")
            }
            EngineError::Camera(_) => {
                AppError::recoverable(ErrorCode::Internal, "Your camera couldn't be used.")
                    .with_hint("You can record without the camera.")
            }

            EngineError::Encode(EncodeError::BinaryNotFound(_)) => AppError::fatal(
                ErrorCode::EncodeMissingBinary,
                "A part of vrecnu is missing, so recordings can't be processed.",
            )
            .with_hint("Reinstalling vrecnu should fix this."),

            // The encoder dying mid-session is the one case that genuinely costs the user
            // their work, so it is the one that reports `session_lost`.
            EngineError::Encode(EncodeError::Died(_) | EncodeError::StdinWrite { .. }) => {
                AppError::fatal(
                    ErrorCode::EncodeDied,
                    "Recording stopped unexpectedly and this recording was lost.",
                )
            }

            EngineError::Encode(_) => AppError::recoverable(
                ErrorCode::EncodeFailed,
                "Processing the recording failed.",
            ),

            EngineError::Publish(PublishError::AllPortsBusy(ports)) => AppError::recoverable(
                ErrorCode::PublishPortsBusy,
                "vrecnu couldn't open the page needed to upload to Drive.",
            )
            .with_hint(format!(
                "Ports {}-{} are all in use. Close whatever is using them and try again, or save a local copy instead.",
                ports[0],
                ports[ports.len() - 1]
            )),

            EngineError::Publish(PublishError::AssetsMissing) => AppError::fatal(
                ErrorCode::PublishAssetsMissing,
                "This build of vrecnu is missing its upload page.",
            )
            .with_hint("Reinstalling vrecnu should fix this."),

            EngineError::Publish(PublishError::TimedOut(_)) => AppError::recoverable(
                ErrorCode::PublishTimeout,
                "The upload page was left open too long and was closed.",
            ),

            EngineError::Publish(_) => {
                AppError::recoverable(ErrorCode::PublishPageError, "Uploading to Drive failed.")
            }

            // The pipe is plumbing, not a device — from the user's chair a broken audio pipe
            // and a broken audio device fail the same way, so it gets the same message and the
            // same "keep going without audio" hint rather than inventing a new error surface.
            EngineError::Pipe(_) => {
                AppError::recoverable(ErrorCode::Internal, "Audio couldn't be recorded.")
                    .with_hint("You can record without audio, or try again.")
            }

            EngineError::InsufficientDisk {
                needed_mb, free_mb, ..
            } => AppError::recoverable(ErrorCode::DiskFull, "There isn't enough free disk space.")
                .with_hint(format!("About {needed_mb} MB is needed and {free_mb} MB is free.")),

            EngineError::IllegalTransition { .. } | EngineError::NoSession => {
                AppError::recoverable(ErrorCode::IllegalState, "That isn't possible right now.")
            }

            EngineError::Io(_) => {
                AppError::recoverable(ErrorCode::Internal, "Something went wrong.")
            }
        };
        base.with_detail(detail)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dead_encoder_reports_the_recording_as_lost() {
        // The distinction the UI hangs severity on.
        let app: AppError = EngineError::Encode(EncodeError::Died("exit 1".into())).into();
        assert_eq!(app.code, ErrorCode::EncodeDied);
        assert!(app.session_lost);
    }

    #[test]
    fn busy_ports_are_recoverable_and_name_the_range() {
        let ports = vrecnu_publish_server::REGISTERED_PORTS;
        let app: AppError = EngineError::Publish(PublishError::AllPortsBusy(ports)).into();
        assert_eq!(app.code, ErrorCode::PublishPortsBusy);
        assert!(!app.session_lost);
        // ADR-002 requires an actionable message rather than a bare failure.
        let hint = app.hint.expect("busy ports must carry a hint");
        assert!(hint.contains("47821"), "{hint}");
        assert!(hint.contains("47825"), "{hint}");
    }

    #[test]
    fn a_busy_camera_tells_the_user_what_to_do() {
        let app: AppError = EngineError::Camera(CameraError::DeviceBusy {
            name: "Integrated Webcam".into(),
        })
        .into();
        assert_eq!(app.code, ErrorCode::CameraBusy);
        assert!(app.hint.is_some());
    }

    #[test]
    fn the_technical_cause_is_kept_as_detail_not_shown_as_the_message() {
        let app: AppError =
            EngineError::Encode(EncodeError::EncoderUnavailable("h264_mf".into())).into();
        // The user reads plain language; the developer detail is still recoverable from logs.
        assert!(!app.message.contains("h264_mf"), "{}", app.message);
        assert!(app.detail.expect("detail").contains("h264_mf"));
    }

    #[test]
    fn user_facing_messages_avoid_jargon() {
        let cases: Vec<EngineError> = vec![
            EngineError::Capture(CaptureError::Unavailable),
            EngineError::Camera(CameraError::NoCamera),
            EngineError::NoSession,
        ];
        for case in cases {
            let app: AppError = case.into();
            for jargon in ["ffmpeg", "WASAPI", "OPFS", "codec", "mutex"] {
                assert!(
                    !app.message.to_lowercase().contains(&jargon.to_lowercase()),
                    "message leaked jargon {jargon:?}: {}",
                    app.message
                );
            }
        }
    }

    #[test]
    fn insufficient_disk_quantifies_the_shortfall() {
        let app: AppError = EngineError::InsufficientDisk {
            needed_mb: 500,
            free_mb: 40,
            path: PathBuf::from("C:/tmp"),
        }
        .into();
        let hint = app.hint.expect("hint");
        assert!(hint.contains("500"));
        assert!(hint.contains("40"));
    }
}
