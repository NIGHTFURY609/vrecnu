//! The FFmpeg sidecar.
//!
//! FFmpeg is a **child process, always** (ADR-004). Linking it via `ffmpeg-next` or any other
//! bindings crate is forbidden: vrecnu is MIT, FFmpeg is LGPL at best and GPL with several
//! common components, and invoking a separate binary is aggregation rather than linking.
//! A crash also kills the child instead of the app, so a failed export loses an export rather
//! than the recording.
//!
//! Two invocations, deliberately:
//!
//! 1. **Live encode**, writing fragmented MP4. `+frag_keyframe+empty_moov` means the file on
//!    disk is playable at *every instant* — if the machine dies mid-recording the user still
//!    has their video. This is the desktop counterpart to ADR-003's "never accumulate in
//!    memory", and it is the single most important flag in the whole command line.
//! 2. **Finalize/edit**, writing `+faststart`. Moving `moov` to the front is what makes
//!    seeking work in Drive's preview player — precisely the failure ADR-003 §3.2 exists to
//!    prevent. When no filters are requested this degrades to `-c copy`, a sub-second remux.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod args;
mod error;
mod process;
mod progress;

pub use args::{live_encode_args, render_args, LiveEncodeParams};
pub use error::EncodeError;
pub use process::{probe_h264_encoder, FfmpegProcess};
pub use progress::{FfmpegProgress, ProgressParser};

/// H.264 encoders vrecnu will use, in preference order (ADR-007).
///
/// `libx264` is deliberately absent. It requires `--enable-gpl`, and ADR-004 §4.1 prefers an
/// LGPL build — a contradiction nobody had noticed because the `configure` line had never
/// been written down. `h264_mf` resolves it: built into FFmpeg on Windows, no external
/// library, hardware-accelerated on any GPU of the last decade, and roughly zero bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum H264Encoder {
    /// Media Foundation, hardware where available. Preferred.
    MediaFoundation,
    /// Software fallback where no MF encoder is present.
    OpenH264,
}

impl H264Encoder {
    /// The `-c:v` argument.
    #[must_use]
    pub const fn ffmpeg_name(self) -> &'static str {
        match self {
            Self::MediaFoundation => "h264_mf",
            Self::OpenH264 => "libopenh264",
        }
    }

    /// Preference order for the startup probe.
    #[must_use]
    pub const fn preference_order() -> [Self; 2] {
        [Self::MediaFoundation, Self::OpenH264]
    }

    /// The `-profile:v` value this encoder will actually accept.
    ///
    /// Not cosmetic. `h264_mf` exposes `profile` as a bare integer with no named constants
    /// registered, so the symbolic `high` that every other H.264 encoder takes fails to parse —
    /// and it fails at *encoder open*, which is after capture has started and the user believes
    /// they are recording. 100 is H.264's High profile IDC, the number `high` resolves to.
    #[must_use]
    pub const fn profile_value(self) -> &'static str {
        match self {
            Self::MediaFoundation => "100",
            Self::OpenH264 => "high",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_foundation_is_preferred() {
        assert_eq!(
            H264Encoder::preference_order()[0],
            H264Encoder::MediaFoundation
        );
    }

    #[test]
    fn media_foundation_asks_for_high_profile_by_number() {
        // `-profile:v high` is rejected by h264_mf at encoder-open time, i.e. after the user
        // has already started recording. This was a real failure, not a hypothetical one.
        assert_eq!(H264Encoder::MediaFoundation.profile_value(), "100");
        assert_eq!(H264Encoder::OpenH264.profile_value(), "high");
    }

    #[test]
    fn libx264_is_never_named() {
        // ADR-007: an LGPL FFmpeg build cannot contain libx264, so nothing may ask for it.
        for encoder in H264Encoder::preference_order() {
            assert_ne!(encoder.ffmpeg_name(), "libx264");
        }
    }
}
