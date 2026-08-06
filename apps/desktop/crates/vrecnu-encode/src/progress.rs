/// One progress report from FFmpeg's `-progress` stream.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct FfmpegProgress {
    /// Frames encoded so far.
    pub frame: u64,
    /// Instantaneous encoding rate.
    pub fps: f32,
    /// Output timestamp in microseconds.
    pub out_time_us: u64,
    /// Frames FFmpeg itself dropped.
    pub drop_frames: u64,
    /// Encoding speed relative to real time; below 1.0 means falling behind.
    pub speed: f32,
    /// True once FFmpeg reports `progress=end`.
    pub finished: bool,
}

/// Parses FFmpeg's `-progress` output.
///
/// `-progress pipe:2` emits stable `key=value` lines terminated by `progress=continue` or
/// `progress=end`. Scraping the human-readable status line instead is the brittle option
/// ADR-004 warns about — its layout changes between releases, which is why the ADR insists on
/// pinning the version. This format has been stable for years and is machine-intended.
#[derive(Debug, Default)]
pub struct ProgressParser {
    current: FfmpegProgress,
}

impl ProgressParser {
    /// A parser with nothing accumulated.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Feeds one line. Returns a snapshot when the line closes a report.
    ///
    /// Values accumulate across lines because FFmpeg emits one key per line and only then a
    /// `progress=` terminator.
    pub fn push_line(&mut self, line: &str) -> Option<FfmpegProgress> {
        let line = line.trim();
        // FFmpeg interleaves banner and warning text with the progress stream; anything
        // without a `key=value` shape is not ours to interpret.
        let (key, value) = line.split_once('=')?;
        let value = value.trim();

        match key.trim() {
            "frame" => self.current.frame = value.parse().unwrap_or(self.current.frame),
            "fps" => self.current.fps = value.parse().unwrap_or(self.current.fps),
            "out_time_us" | "out_time_ms" => {
                // FFmpeg's `out_time_ms` is a misnomer: it is microseconds, like `out_time_us`.
                self.current.out_time_us = value.parse().unwrap_or(self.current.out_time_us);
            }
            "drop_frames" => {
                self.current.drop_frames = value.parse().unwrap_or(self.current.drop_frames);
            }
            "speed" => {
                let cleaned = value.trim_end_matches('x');
                self.current.speed = cleaned.parse().unwrap_or(self.current.speed);
            }
            "progress" => {
                self.current.finished = value == "end";
                return Some(self.current);
            }
            _ => {}
        }
        None
    }

    /// The values accumulated so far.
    #[must_use]
    pub const fn current(&self) -> FfmpegProgress {
        self.current
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_is_emitted_only_on_the_progress_line() {
        let mut parser = ProgressParser::new();
        assert!(parser.push_line("frame=120").is_none());
        assert!(parser.push_line("fps=30.0").is_none());
        let report = parser
            .push_line("progress=continue")
            .expect("report at terminator");
        assert_eq!(report.frame, 120);
        assert!((report.fps - 30.0).abs() < 0.001);
        assert!(!report.finished);
    }

    #[test]
    fn the_end_marker_sets_finished() {
        let mut parser = ProgressParser::new();
        assert!(parser.push_line("progress=end").expect("report").finished);
    }

    #[test]
    fn speed_drops_its_trailing_x() {
        let mut parser = ProgressParser::new();
        parser.push_line("speed=1.02x");
        let report = parser.push_line("progress=continue").expect("report");
        assert!((report.speed - 1.02).abs() < 0.001);
    }

    #[test]
    fn out_time_ms_is_read_as_microseconds() {
        // FFmpeg's key is misnamed; treating it as milliseconds is off by 1000x.
        let mut parser = ProgressParser::new();
        parser.push_line("out_time_ms=5000000");
        let report = parser.push_line("progress=continue").expect("report");
        assert_eq!(report.out_time_us, 5_000_000);
    }

    #[test]
    fn dropped_frames_are_captured_because_the_budget_depends_on_them() {
        let mut parser = ProgressParser::new();
        parser.push_line("drop_frames=3");
        let report = parser.push_line("progress=continue").expect("report");
        assert_eq!(report.drop_frames, 3);
    }

    #[test]
    fn unparseable_values_keep_the_previous_reading() {
        // A garbled line must not reset a counter to zero and fake a recovery.
        let mut parser = ProgressParser::new();
        parser.push_line("frame=99");
        parser.push_line("frame=not-a-number");
        assert_eq!(parser.current().frame, 99);
    }

    #[test]
    fn noise_without_an_equals_sign_is_ignored() {
        let mut parser = ProgressParser::new();
        assert!(parser.push_line("some banner text").is_none());
        assert!(parser.push_line("").is_none());
    }
}
