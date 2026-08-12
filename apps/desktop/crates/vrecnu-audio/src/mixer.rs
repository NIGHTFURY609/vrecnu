//! Combines up to two PCM sources — microphone and system-audio loopback — into the single
//! stream `LiveEncodeParams::audio_pipe` expects.
//!
//! Each source gets its own [`SilenceFiller`], reused unchanged from the single-source case:
//! silence-filling one stream and silence-filling two streams that are then summed is the same
//! problem solved twice, not a new one. The one property this module adds is that **both
//! fillers are always ticked with the same `elapsed_ns`**, which — see
//! [`SourceState::take`]'s doc comment — guarantees the two streams are always exactly the same
//! length, so summing them never has to guess what to do with a length mismatch.

use std::collections::VecDeque;

use crossbeam_channel::{bounded, Receiver, Sender};

use crate::{PcmSink, SilenceFiller, BYTES_PER_FRAME};

/// How many pending packets a source's queue holds before the mixer's writer thread is
/// considered behind.
///
/// Audio packets are tiny next to a video frame — a WASAPI period is typically ~10 ms, a few
/// hundred bytes at 48 kHz stereo `s16` — so 64 packets is several seconds of headroom without
/// meaningfully affecting memory.
const QUEUE_CAPACITY: usize = 64;

/// Which of the two possible sources are active for this recording.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MixerSources {
    /// Microphone only.
    Mic,
    /// System-audio loopback only.
    System,
    /// Both, summed together.
    Both,
}

/// Feeds PCM from one [`crate::AudioSource`] into a [`Mixer`].
///
/// Handed to `AudioSource::start` by whoever constructs the mixer.
pub struct MixerSink {
    tx: Sender<Vec<u8>>,
}

impl PcmSink for MixerSink {
    fn on_pcm(&mut self, samples: &[u8], _qpc: i64) {
        // A full queue means the tick thread has stalled; dropping the packet is the same
        // backpressure choice the video frame pool makes when it's exhausted (see
        // `vrecnu-engine::pipeline`) — losing a few milliseconds of audio is recoverable,
        // blocking the WASAPI callback thread is not, and it must not block: same contract as
        // `PcmSink::on_pcm`'s doc comment already states.
        let _ = self.tx.try_send(samples.to_vec());
    }
}

/// The sinks a freshly built [`Mixer`] hands out, one per active source, for the caller to pass
/// to the matching `AudioSource::start`.
#[derive(Default)]
pub struct MixerSinks {
    /// Present when the mixer was built with [`MixerSources::Mic`] or [`MixerSources::Both`].
    pub mic: Option<Box<dyn PcmSink>>,
    /// Present when the mixer was built with [`MixerSources::System`] or [`MixerSources::Both`].
    pub system: Option<Box<dyn PcmSink>>,
}

struct SourceState {
    filler: SilenceFiller,
    rx: Receiver<Vec<u8>>,
    pending: VecDeque<u8>,
}

impl SourceState {
    fn new() -> (Self, MixerSink) {
        let (tx, rx) = bounded(QUEUE_CAPACITY);
        (
            Self {
                filler: SilenceFiller::new(),
                rx,
                pending: VecDeque::new(),
            },
            MixerSink { tx },
        )
    }

    /// Bytes due by `elapsed_ns`: real audio first, silence padding the remainder.
    ///
    /// Always returns exactly `SilenceFiller::shortfall_bytes(elapsed_ns)` bytes, computed
    /// *before* this call — real or silent, the total is the same, which is the property
    /// [`Mixer::tick`] relies on to sum two sources without a length mismatch.
    fn take(&mut self, elapsed_ns: u64) -> Vec<u8> {
        while let Ok(chunk) = self.rx.try_recv() {
            self.pending.extend(chunk);
        }

        let want = self.filler.shortfall_bytes(elapsed_ns);
        // Both operands are already frame-aligned (resample.rs only ever emits whole frames),
        // so this floor is redundant in practice — kept because a torn packet degrading into a
        // shifted channel order is a worse failure than a few defensive bytes of code.
        let take_real = want.min(self.pending.len());
        let take_real = take_real - (take_real % BYTES_PER_FRAME);

        let mut out: Vec<u8> = self.pending.drain(..take_real).collect();
        self.filler.account_for(out.len());

        let silence_needed = self.filler.shortfall_bytes(elapsed_ns);
        out.resize(out.len() + silence_needed, 0);
        self.filler.account_for(silence_needed);

        out
    }
}

/// Combines the active source(s) into one 48 kHz stereo `s16le` stream.
pub struct Mixer {
    mic: Option<SourceState>,
    system: Option<SourceState>,
}

impl Mixer {
    /// Builds a mixer for `sources` and the sinks its `AudioSource`s should write into.
    #[must_use]
    pub fn new(sources: MixerSources) -> (Self, MixerSinks) {
        let mut mic_state = None;
        let mut mic_sink = None;
        if matches!(sources, MixerSources::Mic | MixerSources::Both) {
            let (state, sink) = SourceState::new();
            mic_state = Some(state);
            mic_sink = Some(Box::new(sink) as Box<dyn PcmSink>);
        }

        let mut system_state = None;
        let mut system_sink = None;
        if matches!(sources, MixerSources::System | MixerSources::Both) {
            let (state, sink) = SourceState::new();
            system_state = Some(state);
            system_sink = Some(Box::new(sink) as Box<dyn PcmSink>);
        }

        (
            Self {
                mic: mic_state,
                system: system_state,
            },
            MixerSinks {
                mic: mic_sink,
                system: system_sink,
            },
        )
    }

    /// The mixed bytes due by `elapsed_ns` since the recording's shared clock started.
    ///
    /// One active source passes through unchanged (still silence-filled). Two are summed with a
    /// saturating add, so a loud simultaneous mic-plus-system moment clips at full scale instead
    /// of wrapping into noise the way a plain `wrapping_add` would.
    #[must_use]
    pub fn tick(&mut self, elapsed_ns: u64) -> Vec<u8> {
        match (&mut self.mic, &mut self.system) {
            (Some(mic), Some(system)) => {
                let mic_bytes = mic.take(elapsed_ns);
                let system_bytes = system.take(elapsed_ns);
                mix_saturating(&mic_bytes, &system_bytes)
            }
            (Some(mic), None) => mic.take(elapsed_ns),
            (None, Some(system)) => system.take(elapsed_ns),
            (None, None) => Vec::new(),
        }
    }
}

/// Sums two equal-length interleaved `s16le` buffers sample-by-sample.
///
/// `a` and `b` are guaranteed equal length by [`SourceState::take`]'s contract (both fillers are
/// always ticked with the same `elapsed_ns` and start from the same zero). Mixing over the
/// shorter length regardless is the same "degrade rather than panic" choice the rest of this
/// codebase makes for an invariant that should always hold but costs nothing to double-check.
fn mix_saturating(a: &[u8], b: &[u8]) -> Vec<u8> {
    debug_assert_eq!(a.len(), b.len(), "mixer sources drifted out of lockstep");
    let len = a.len().min(b.len());
    let len = len - (len % 2); // whole i16 samples only
    let mut out = Vec::with_capacity(len);
    for i in (0..len).step_by(2) {
        let sa = i16::from_le_bytes([a[i], a[i + 1]]);
        let sb = i16::from_le_bytes([b[i], b[i + 1]]);
        out.extend_from_slice(&sa.saturating_add(sb).to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn one_second_ns() -> u64 {
        1_000_000_000
    }

    fn s16_bytes(samples: &[i16]) -> Vec<u8> {
        samples.iter().flat_map(|s| s.to_le_bytes()).collect()
    }

    #[test]
    fn mic_only_silence_fills_when_nothing_arrives() {
        let (mut mixer, sinks) = Mixer::new(MixerSources::Mic);
        assert!(sinks.mic.is_some());
        assert!(sinks.system.is_none());
        let out = mixer.tick(one_second_ns());
        assert_eq!(out.len(), 48_000 * usize::from(crate::CHANNELS) * 2);
        assert!(out.iter().all(|&b| b == 0));
    }

    #[test]
    fn system_only_silence_fills_when_nothing_arrives() {
        let (mut mixer, sinks) = Mixer::new(MixerSources::System);
        assert!(sinks.mic.is_none());
        assert!(sinks.system.is_some());
        let out = mixer.tick(one_second_ns());
        assert_eq!(out.len(), 48_000 * usize::from(crate::CHANNELS) * 2);
    }

    #[test]
    fn real_audio_is_used_when_available() {
        let (mut mixer, sinks) = Mixer::new(MixerSources::Mic);
        let mut sink = sinks.mic.expect("mic sink present");
        let real = s16_bytes(&[1234, -1234]); // one stereo frame
        sink.on_pcm(&real, 0);

        let out = mixer.tick(one_second_ns());
        // The real frame leads, the rest of the second is silence.
        assert_eq!(&out[..4], real.as_slice());
        assert!(out[4..].iter().all(|&b| b == 0));
    }

    #[test]
    fn both_active_sums_real_audio() {
        let (mut mixer, sinks) = Mixer::new(MixerSources::Both);
        let mut mic_sink = sinks.mic.expect("mic sink present");
        let mut system_sink = sinks.system.expect("system sink present");
        mic_sink.on_pcm(&s16_bytes(&[1000, -1000]), 0);
        system_sink.on_pcm(&s16_bytes(&[500, 500]), 0);

        let out = mixer.tick(one_second_ns());
        let first_frame: Vec<i16> = out[..4]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        assert_eq!(first_frame, vec![1500, -500]);
    }

    #[test]
    fn saturating_add_clips_instead_of_wrapping() {
        let (mut mixer, sinks) = Mixer::new(MixerSources::Both);
        let mut mic_sink = sinks.mic.expect("mic sink present");
        let mut system_sink = sinks.system.expect("system sink present");
        mic_sink.on_pcm(&s16_bytes(&[30_000, i16::MIN]), 0);
        system_sink.on_pcm(&s16_bytes(&[30_000, i16::MIN]), 0);

        let out = mixer.tick(one_second_ns());
        let first_frame: Vec<i16> = out[..4]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();
        // 30_000 + 30_000 would wrap past i16::MAX; MIN + MIN would wrap past i16::MIN.
        assert_eq!(first_frame, vec![i16::MAX, i16::MIN]);
    }

    #[test]
    fn mic_and_system_stay_the_same_length_across_uneven_arrivals() {
        let (mut mixer, sinks) = Mixer::new(MixerSources::Both);
        let mut mic_sink = sinks.mic.expect("mic sink present");
        let system_sink = sinks.system.expect("system sink present");
        // Only the mic ever delivers real audio; system stays fully silent throughout. The
        // invariant under test is length, not content — mismatched arrival is the whole point.
        mic_sink.on_pcm(&s16_bytes(&vec![777; 2 * 4_000]), 0);
        drop(system_sink);

        for tick in 1..=5u64 {
            let out = mixer.tick(tick * 100_000_000); // every 100ms
            assert_eq!(out.len() % 4, 0);
        }
    }

    #[test]
    fn a_full_queue_drops_the_newest_packet_instead_of_blocking() {
        let (_mixer, sinks) = Mixer::new(MixerSources::Mic);
        let mut sink = sinks.mic.expect("mic sink present");
        // One over capacity: must not panic or block, must simply drop the last one.
        for _ in 0..=QUEUE_CAPACITY {
            sink.on_pcm(&s16_bytes(&[1, 1]), 0);
        }
    }

    #[test]
    fn neither_source_active_produces_nothing() {
        let (mut mixer, sinks) = Mixer::new(MixerSources::Mic);
        assert!(sinks.system.is_none());
        // Constructing Both then only using one is exercised elsewhere; this just confirms the
        // no-source path some future caller might reach doesn't panic.
        let mut empty = Mixer {
            mic: None,
            system: None,
        };
        assert!(empty.tick(one_second_ns()).is_empty());
        let _ = mixer.tick(0); // keep `mixer` used
    }
}
