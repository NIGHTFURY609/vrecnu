//! Format conversion.
//!
//! WASAPI hands back whatever the device's mix format happens to be — 44.1 kHz or 48 kHz,
//! mono or stereo or more, `f32` or `s16`. Everything downstream of capture (the mixer, the
//! FFmpeg pipe) assumes 48 kHz stereo `s16le` — see [`crate::SAMPLE_RATE_HZ`] /
//! [`crate::CHANNELS`]. This module is where that assumption is made true, once, at the
//! source, so nothing downstream ever has to think about sample rate or channel count again.

use crate::{CHANNELS, SAMPLE_RATE_HZ};

/// A device's native sample format, as reported by `IAudioClient::GetMixFormat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SourceFormat {
    /// Native sample rate in Hz.
    pub sample_rate_hz: u32,
    /// Native channel count. 1 is duplicated to stereo; more than 2 is truncated to the first
    /// two — surround capture is out of scope (see the design spec's explicit scope cuts).
    pub channels: u16,
    /// Whether samples arrive as `f32` or `s16`.
    pub sample_kind: SampleKind,
}

/// The two sample encodings WASAPI mix formats use in practice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleKind {
    /// IEEE float, one `f32` per sample, nominally in `[-1.0, 1.0]`.
    F32,
    /// Signed 16-bit PCM, little-endian.
    S16,
}

impl SampleKind {
    const fn bytes_per_sample(self) -> usize {
        match self {
            Self::F32 => 4,
            Self::S16 => 2,
        }
    }
}

/// Converts a buffer of raw device samples into interleaved 48 kHz stereo `s16le`.
///
/// `input` is interleaved samples in `format`'s native layout. A trailing partial frame (fewer
/// bytes than one full frame) is dropped rather than read out of bounds — real WASAPI buffers
/// are always frame-aligned, but a malformed input degrading silently is safer than one that
/// panics (`panic = "abort"` in release takes the whole recording down with it).
#[must_use]
pub fn to_48k_stereo_s16(input: &[u8], format: SourceFormat) -> Vec<u8> {
    let stereo = decode_to_stereo_f32(input, format);
    let resampled = resample_linear(&stereo, format.sample_rate_hz, SAMPLE_RATE_HZ);
    encode_s16(&resampled)
}

/// Decodes native-format bytes into stereo `f32` frames, duplicating mono and truncating any
/// channel beyond the first two.
fn decode_to_stereo_f32(input: &[u8], format: SourceFormat) -> Vec<(f32, f32)> {
    let bytes_per_sample = format.sample_kind.bytes_per_sample();
    let channels = format.channels.max(1) as usize;
    let frame_bytes = bytes_per_sample * channels;
    if frame_bytes == 0 {
        return Vec::new();
    }
    let frame_count = input.len() / frame_bytes;

    let mut out = Vec::with_capacity(frame_count);
    for frame_idx in 0..frame_count {
        let frame_start = frame_idx * frame_bytes;
        let sample_at = |channel: usize| -> f32 {
            let offset = frame_start + channel * bytes_per_sample;
            decode_sample(
                &input[offset..offset + bytes_per_sample],
                format.sample_kind,
            )
        };
        let (l, r) = if channels == 1 {
            let v = sample_at(0);
            (v, v)
        } else {
            (sample_at(0), sample_at(1))
        };
        out.push((l, r));
    }
    out
}

fn decode_sample(bytes: &[u8], kind: SampleKind) -> f32 {
    match kind {
        SampleKind::F32 => f32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        // 32768.0, not 32767.0: i16::MIN maps to exactly -1.0. The asymmetric encode side
        // accepts a possible ±1 LSB error at the extreme rather than ever producing a value
        // outside [-1.0, 1.0] here.
        SampleKind::S16 => f32::from(i16::from_le_bytes([bytes[0], bytes[1]])) / 32_768.0,
    }
}

/// Linear-interpolation resample. Simple on purpose — this is voice/system-audio at telephony-
/// to-CD rates, not a mastering pipeline, and pulling in a full resampling crate (`rubato` et
/// al.) would spend real bytes of the binary-size budget (AGENTS.md §1) on quality nobody will
/// hear the difference of.
fn resample_linear(input: &[(f32, f32)], src_rate: u32, dst_rate: u32) -> Vec<(f32, f32)> {
    if input.is_empty() {
        return Vec::new();
    }
    if src_rate == dst_rate {
        return input.to_vec();
    }

    let ratio = f64::from(src_rate) / f64::from(dst_rate);
    // Ceiling, not floor/round: undershooting here means a resample step can hand the mixer
    // fewer bytes than SilenceFiller's accounting expects, which reintroduces the exact
    // audio/video drift this whole module exists to prevent.
    let out_len = ((input.len() as f64) / ratio).ceil() as usize;

    let mut out = Vec::with_capacity(out_len);
    let last = input.len() - 1;
    for i in 0..out_len {
        let src_pos = i as f64 * ratio;
        let idx0 = (src_pos.floor() as usize).min(last);
        let idx1 = (idx0 + 1).min(last);
        let frac = (src_pos - idx0 as f64) as f32;

        let (l0, r0) = input[idx0];
        let (l1, r1) = input[idx1];
        out.push((l0 + (l1 - l0) * frac, r0 + (r1 - r0) * frac));
    }
    out
}

fn encode_s16(frames: &[(f32, f32)]) -> Vec<u8> {
    let mut out = Vec::with_capacity(frames.len() * usize::from(CHANNELS) * 2);
    for &(l, r) in frames {
        out.extend_from_slice(&encode_sample(l));
        out.extend_from_slice(&encode_sample(r));
    }
    out
}

fn encode_sample(v: f32) -> [u8; 2] {
    let clamped = v.clamp(-1.0, 1.0);
    let scaled = (clamped * 32_767.0).round();
    (scaled as i16).to_le_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s16_bytes(samples: &[i16]) -> Vec<u8> {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    fn stereo_s16_format(rate: u32) -> SourceFormat {
        SourceFormat {
            sample_rate_hz: rate,
            channels: 2,
            sample_kind: SampleKind::S16,
        }
    }

    #[test]
    fn stereo_s16_at_target_rate_passes_through_within_one_lsb() {
        let input = s16_bytes(&[100, -200, 300, -400]);
        let out = to_48k_stereo_s16(&input, stereo_s16_format(SAMPLE_RATE_HZ));
        let decoded: Vec<i16> = out
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        for (original, round_tripped) in [100i16, -200, 300, -400].iter().zip(decoded) {
            assert!(
                (i32::from(*original) - i32::from(round_tripped)).abs() <= 1,
                "expected {original}, got {round_tripped}"
            );
        }
    }

    #[test]
    fn mono_is_duplicated_to_both_stereo_channels() {
        let format = SourceFormat {
            sample_rate_hz: SAMPLE_RATE_HZ,
            channels: 1,
            sample_kind: SampleKind::S16,
        };
        let input = s16_bytes(&[1000, 2000]); // two mono frames
        let out = to_48k_stereo_s16(&input, format);
        let decoded: Vec<i16> = out
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        // Frame 0: (1000, 1000); frame 1: (2000, 2000).
        assert_eq!(decoded, vec![1000, 1000, 2000, 2000]);
    }

    #[test]
    fn extra_channels_beyond_stereo_are_dropped() {
        let format = SourceFormat {
            sample_rate_hz: SAMPLE_RATE_HZ,
            channels: 4,
            sample_kind: SampleKind::S16,
        };
        // One 4-channel frame: L, R, and two channels that must be ignored.
        let input = s16_bytes(&[500, -500, 9999, -9999]);
        let out = to_48k_stereo_s16(&input, format);
        let decoded: Vec<i16> = out
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(decoded, vec![500, -500]);
    }

    #[test]
    fn upsampling_produces_roughly_double_the_frames() {
        let format = SourceFormat {
            sample_rate_hz: 24_000,
            channels: 2,
            sample_kind: SampleKind::S16,
        };
        let input = s16_bytes(&vec![1000i16; 2 * 100]); // 100 stereo frames at 24 kHz
        let out = to_48k_stereo_s16(&input, format);
        let out_frames = out.len() / 4; // 4 bytes per stereo s16 frame
        assert_eq!(out_frames, 200);
    }

    #[test]
    fn downsampling_produces_roughly_half_the_frames() {
        let format = SourceFormat {
            sample_rate_hz: 96_000,
            channels: 2,
            sample_kind: SampleKind::S16,
        };
        let input = s16_bytes(&vec![1000i16; 2 * 200]); // 200 stereo frames at 96 kHz
        let out = to_48k_stereo_s16(&input, format);
        let out_frames = out.len() / 4;
        assert_eq!(out_frames, 100);
    }

    #[test]
    fn f32_beyond_unity_is_clamped_not_wrapped() {
        let format = SourceFormat {
            sample_rate_hz: SAMPLE_RATE_HZ,
            channels: 2,
            sample_kind: SampleKind::F32,
        };
        // 1.5 and -1.5 are out of the nominal [-1, 1] range a buggy/clipping source could emit.
        let input: Vec<u8> = [1.5f32, -1.5f32]
            .iter()
            .flat_map(|f| f.to_le_bytes())
            .collect();
        let out = to_48k_stereo_s16(&input, format);
        let decoded: Vec<i16> = out
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(decoded, vec![i16::MAX, -i16::MAX]);
    }

    #[test]
    fn empty_input_produces_empty_output() {
        let out = to_48k_stereo_s16(&[], stereo_s16_format(SAMPLE_RATE_HZ));
        assert!(out.is_empty());
    }

    #[test]
    fn a_short_trailing_frame_is_dropped_rather_than_read_out_of_bounds() {
        let mut input = s16_bytes(&[100, -100]); // one full stereo frame
        input.push(0); // one stray trailing byte, not a full sample
        let out = to_48k_stereo_s16(&input, stereo_s16_format(SAMPLE_RATE_HZ));
        assert_eq!(out.len(), 4); // exactly the one complete frame, s16 stereo
    }
}
