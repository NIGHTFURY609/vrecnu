use crate::{BYTES_PER_FRAME, SAMPLE_RATE_HZ};

/// Keeps the audio timeline aligned with wall-clock time.
///
/// WASAPI loopback stops delivering packets entirely when nothing is playing. Feeding only
/// what arrives produces an audio stream shorter than the video, and the gap compounds across
/// every silent stretch — a 40-minute recording with intermittent audio can finish seconds
/// out of sync. This tracks how many samples *should* exist by now and reports the shortfall
/// so the caller can write zeros.
#[derive(Debug)]
pub struct SilenceFiller {
    samples_written: u64,
}

impl Default for SilenceFiller {
    fn default() -> Self {
        Self::new()
    }
}

impl SilenceFiller {
    /// Starts at the beginning of the timeline.
    #[must_use]
    pub const fn new() -> Self {
        Self { samples_written: 0 }
    }

    /// Records that `bytes` of real audio were written.
    pub fn account_for(&mut self, bytes: usize) {
        self.samples_written += (bytes / BYTES_PER_FRAME) as u64;
    }

    /// Sample frames that should exist by `elapsed_ns`.
    #[must_use]
    pub const fn expected_samples(elapsed_ns: u64) -> u64 {
        elapsed_ns * SAMPLE_RATE_HZ as u64 / 1_000_000_000
    }

    /// Bytes of silence needed to reach `elapsed_ns`, or zero when already caught up.
    #[must_use]
    pub const fn shortfall_bytes(&self, elapsed_ns: u64) -> usize {
        let expected = Self::expected_samples(elapsed_ns);
        if expected <= self.samples_written {
            return 0;
        }
        (expected - self.samples_written) as usize * BYTES_PER_FRAME
    }

    /// Sample frames written so far, real and silent.
    #[must_use]
    pub const fn samples_written(&self) -> u64 {
        self.samples_written
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_second_expects_48000_samples() {
        assert_eq!(SilenceFiller::expected_samples(1_000_000_000), 48_000);
    }

    #[test]
    fn a_silent_second_reports_a_full_second_of_shortfall() {
        // Nothing played, so nothing arrived: 48 000 frames x 4 bytes must be filled.
        let filler = SilenceFiller::new();
        assert_eq!(filler.shortfall_bytes(1_000_000_000), 192_000);
    }

    #[test]
    fn real_audio_reduces_the_shortfall() {
        let mut filler = SilenceFiller::new();
        filler.account_for(96_000); // half a second of real audio
        assert_eq!(filler.shortfall_bytes(1_000_000_000), 96_000);
    }

    #[test]
    fn a_stream_running_ahead_never_reports_a_negative_shortfall() {
        let mut filler = SilenceFiller::new();
        filler.account_for(400_000);
        assert_eq!(filler.shortfall_bytes(1_000_000_000), 0);
    }

    #[test]
    fn silence_counts_toward_the_timeline_once_accounted_for() {
        let mut filler = SilenceFiller::new();
        let gap = filler.shortfall_bytes(1_000_000_000);
        filler.account_for(gap);
        assert_eq!(filler.shortfall_bytes(1_000_000_000), 0);
        assert_eq!(filler.samples_written(), 48_000);
    }
}
