//! System-audio and microphone capture.
//!
//! Carries the same hazard as video capture, in a different costume: **WASAPI loopback
//! delivers no packets while nothing is playing.** A recording made against a silent desktop
//! therefore produces an audio track shorter than the video, and the two drift apart with
//! every quiet moment. The fix is the same shape as the video pacer — fill the gap with
//! silence measured against elapsed QPC rather than against packets received.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod mixer;
mod resample;
mod silence;
#[cfg(windows)]
mod wasapi_source;

pub use error::AudioError;
pub use mixer::{Mixer, MixerSink, MixerSinks, MixerSources};
pub use resample::{to_48k_stereo_s16, SampleKind, SourceFormat};
pub use silence::SilenceFiller;
#[cfg(windows)]
pub use wasapi_source::{default_devices_available, WasapiLoopbackSource, WasapiMicSource};

/// Everything downstream assumes this format, so conversion happens at the source.
pub const SAMPLE_RATE_HZ: u32 = 48_000;
/// Stereo. FFmpeg is told `-ac 2` to match.
pub const CHANNELS: u16 = 2;
/// Bytes per interleaved stereo frame of `s16le`.
pub const BYTES_PER_FRAME: usize = 2 * 2;

/// Receives PCM as it arrives.
pub trait PcmSink: Send {
    /// Interleaved 48 kHz stereo `s16le`. Called on an audio callback thread, so it must not
    /// block — the same constraint the video capture callback is under.
    fn on_pcm(&mut self, samples: &[u8], qpc: i64);
}

/// A capture endpoint: a microphone, or an output device opened in loopback mode.
pub trait AudioSource: Send {
    /// Begins delivering PCM to `sink`.
    fn start(&mut self, sink: Box<dyn PcmSink>) -> Result<(), AudioError>;
    /// Stops delivery.
    fn stop(&mut self) -> Result<(), AudioError>;
}

/// Converts a linear peak amplitude to dBFS for the level meters.
///
/// Digital silence is negative infinity, which no meter can draw, so it is clamped to -100 dB.
#[must_use]
pub fn amplitude_to_dbfs(peak: f32) -> f32 {
    if peak <= 0.000_01 {
        return -100.0;
    }
    20.0 * peak.log10()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_scale_is_zero_dbfs() {
        assert!((amplitude_to_dbfs(1.0) - 0.0).abs() < 0.001);
    }

    #[test]
    fn half_scale_is_about_minus_six_db() {
        assert!((amplitude_to_dbfs(0.5) + 6.02).abs() < 0.01);
    }

    #[test]
    fn silence_is_clamped_rather_than_infinite() {
        // -inf would break every meter that tries to draw it.
        assert_eq!(amplitude_to_dbfs(0.0), -100.0);
    }

    #[test]
    fn a_stereo_frame_is_four_bytes() {
        assert_eq!(BYTES_PER_FRAME, 4);
        // One second of 48 kHz stereo s16 is 192 000 bytes; the silence filler depends on it.
        assert_eq!(SAMPLE_RATE_HZ as usize * BYTES_PER_FRAME, 192_000);
    }
}
