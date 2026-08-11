/// What the pacer decided on one tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PaceDecision {
    /// A new frame arrived; send it.
    SendNew,
    /// Nothing arrived, so re-send the previous frame to keep the timeline honest.
    Duplicate,
    /// Nothing arrived and there is no previous frame yet.
    Skip,
}

/// Converts the OS's change-driven frame delivery into a constant frame rate.
///
/// Windows Graphics Capture is event-driven: no change, no frame. FFmpeg's `rawvideo` input
/// is constant-rate: every frame it receives advances the timeline by exactly `1/fps`.
/// Bridging those two models is not optional — skipping it yields a recording whose
/// wall-clock length is shorter than the session, with no error raised anywhere.
#[derive(Debug)]
pub struct Pacer {
    frame_interval_ns: u64,
    next_tick_ns: u64,
    has_previous: bool,
    frames_sent: u64,
    frames_duplicated: u64,
}

impl Pacer {
    /// Creates a pacer for `fps`, with its first deadline at `start_ns` on the QPC timeline.
    ///
    /// # Panics
    /// Panics if `fps` is zero.
    #[must_use]
    pub fn new(fps: u32, start_ns: u64) -> Self {
        assert!(fps > 0, "fps must be non-zero");
        Self {
            frame_interval_ns: 1_000_000_000 / u64::from(fps),
            next_tick_ns: start_ns,
            has_previous: false,
            frames_sent: 0,
            frames_duplicated: 0,
        }
    }

    /// Nanoseconds between frames.
    #[must_use]
    pub const fn frame_interval_ns(&self) -> u64 {
        self.frame_interval_ns
    }

    /// When the next frame is due, on the QPC timeline.
    #[must_use]
    pub const fn next_tick_ns(&self) -> u64 {
        self.next_tick_ns
    }

    /// Advances one tick and reports what to send.
    ///
    /// The deadline advances by exactly one interval rather than being reset to "now", so
    /// scheduler jitter cannot accumulate into drift over a long recording.
    pub fn tick(&mut self, new_frame_available: bool) -> PaceDecision {
        self.next_tick_ns = self.next_tick_ns.wrapping_add(self.frame_interval_ns);

        if new_frame_available {
            self.has_previous = true;
            self.frames_sent += 1;
            PaceDecision::SendNew
        } else if self.has_previous {
            self.frames_sent += 1;
            self.frames_duplicated += 1;
            PaceDecision::Duplicate
        } else {
            PaceDecision::Skip
        }
    }

    /// Frames handed to the encoder, duplicates included.
    #[must_use]
    pub const fn frames_sent(&self) -> u64 {
        self.frames_sent
    }

    /// How many of those were duplicates. Expected on a static screen; not a fault.
    #[must_use]
    pub const fn frames_duplicated(&self) -> u64 {
        self.frames_duplicated
    }

    /// Ticks owed by `now_ns`, for a loop catching up after a scheduling stall.
    #[must_use]
    pub const fn ticks_due(&self, now_ns: u64) -> u64 {
        if now_ns < self.next_tick_ns {
            return 0;
        }
        (now_ns - self.next_tick_ns) / self.frame_interval_ns + 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn thirty_fps_is_a_33_ms_interval() {
        assert_eq!(Pacer::new(30, 0).frame_interval_ns(), 33_333_333);
    }

    #[test]
    fn a_static_screen_still_produces_a_full_timeline() {
        // The whole point of this type. Ten seconds of an unchanging desktop at 30 fps must
        // still be 300 frames, or the file comes out shorter than the session the user just
        // recorded — and nothing else in the pipeline would notice.
        let mut pacer = Pacer::new(30, 0);
        assert_eq!(pacer.tick(true), PaceDecision::SendNew);
        for _ in 0..299 {
            assert_eq!(pacer.tick(false), PaceDecision::Duplicate);
        }
        assert_eq!(pacer.frames_sent(), 300);
        assert_eq!(pacer.frames_duplicated(), 299);
    }

    #[test]
    fn nothing_is_emitted_before_the_first_real_frame() {
        // Duplicating a frame that does not exist would put uninitialised bytes at the head
        // of the file.
        let mut pacer = Pacer::new(30, 0);
        assert_eq!(pacer.tick(false), PaceDecision::Skip);
        assert_eq!(pacer.frames_sent(), 0);
    }

    #[test]
    fn deadlines_advance_by_exactly_one_interval() {
        // Resetting to "now" would let jitter accumulate into A/V drift over a long session.
        let mut pacer = Pacer::new(30, 1_000);
        pacer.tick(true);
        assert_eq!(pacer.next_tick_ns(), 1_000 + 33_333_333);
        pacer.tick(true);
        assert_eq!(pacer.next_tick_ns(), 1_000 + 66_666_666);
    }

    #[test]
    fn a_stall_reports_every_owed_tick() {
        let pacer = Pacer::new(30, 0);
        assert_eq!(pacer.ticks_due(0), 1);
        assert_eq!(pacer.ticks_due(100_000_000), 4);
    }

    #[test]
    fn no_ticks_are_owed_before_the_deadline() {
        let pacer = Pacer::new(30, 1_000_000_000);
        assert_eq!(pacer.ticks_due(0), 0);
    }

    #[test]
    fn a_mixed_session_counts_real_and_duplicated_frames_separately() {
        let mut pacer = Pacer::new(30, 0);
        pacer.tick(true);
        pacer.tick(false);
        pacer.tick(true);
        pacer.tick(false);
        assert_eq!(pacer.frames_sent(), 4);
        assert_eq!(pacer.frames_duplicated(), 2);
    }
}
