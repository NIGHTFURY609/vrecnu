//! Process lifetime for the FFmpeg sidecar.
//!
//! Three threads' worth of concern in one type. The caller writes frames on its own thread; a
//! reader thread drains stderr so the `-progress` stream stays current *and* so the pipe never
//! fills; and FFmpeg itself does the work. Draining stderr is not optional politeness — a full
//! stderr pipe blocks the child, which stalls its stdin reads, which blocks our frame writes,
//! and the recording quietly stops advancing with no error anywhere.
//!
//! Shutdown is by **closing stdin**, never by killing. `-f rawvideo -i pipe:0` ends at EOF, at
//! which point FFmpeg flushes the final fragment and exits 0. Killing it instead truncates the
//! file mid-fragment — which the `+frag_keyframe` flags survive, but only by losing the tail
//! that the user just recorded.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use crate::{EncodeError, FfmpegProgress, H264Encoder, ProgressParser};

/// How many stderr lines to keep for an error message. FFmpeg explains a failure in its last
/// handful of lines; keeping the whole stream would mean holding an unbounded log in memory for
/// a five-hour recording.
const STDERR_TAIL_LINES: usize = 24;

/// Suppresses the console window Windows would otherwise flash for a child process.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Shared between the caller and the stderr reader thread.
#[derive(Debug, Default)]
struct Diagnostics {
    progress: Mutex<FfmpegProgress>,
    stderr_tail: Mutex<VecDeque<String>>,
}

impl Diagnostics {
    fn record_line(&self, line: &str) {
        let mut tail = self.stderr_tail.lock().expect("stderr tail mutex poisoned");
        if tail.len() == STDERR_TAIL_LINES {
            tail.pop_front();
        }
        tail.push_back(line.to_string());
    }

    fn tail(&self) -> String {
        self.stderr_tail
            .lock()
            .expect("stderr tail mutex poisoned")
            .iter()
            .cloned()
            .collect::<Vec<_>>()
            .join("; ")
    }
}

/// A running `ffmpeg` child process.
#[derive(Debug)]
pub struct FfmpegProcess {
    child: Child,
    /// `Option` so [`FfmpegProcess::finish`] can drop it to signal EOF while still owning `self`.
    stdin: Option<ChildStdin>,
    reader: Option<JoinHandle<()>>,
    diagnostics: Arc<Diagnostics>,
    frames_written: u64,
}

impl FfmpegProcess {
    /// Spawns `ffmpeg` with `args`, wiring stdin for frames and stderr for `-progress`.
    ///
    /// # Errors
    /// [`EncodeError::BinaryNotFound`] when `binary` cannot be executed, [`EncodeError::Died`]
    /// for any other spawn failure.
    pub fn spawn(binary: &Path, args: &[String]) -> Result<Self, EncodeError> {
        let mut command = Command::new(binary);
        command
            .args(args)
            .stdin(Stdio::piped())
            // FFmpeg writes nothing useful to stdout here (the output is a file) but leaving it
            // inherited would interleave with our own logs.
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(CREATE_NO_WINDOW);
        }

        let mut child = command.spawn().map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                EncodeError::BinaryNotFound(PathBuf::from(binary))
            } else {
                EncodeError::Died(err.to_string())
            }
        })?;

        let stdin = child.stdin.take();
        let stderr = child.stderr.take();
        let diagnostics = Arc::new(Diagnostics::default());

        let reader = stderr.map(|stderr| {
            let diagnostics = Arc::clone(&diagnostics);
            std::thread::Builder::new()
                .name("vrecnu-ffmpeg-stderr".into())
                .spawn(move || drain_stderr(stderr, &diagnostics))
                .expect("spawning the ffmpeg stderr reader")
        });

        Ok(Self {
            child,
            stdin,
            reader,
            diagnostics,
            frames_written: 0,
        })
    }

    /// Writes one tightly-packed frame to FFmpeg's stdin.
    ///
    /// # Errors
    /// [`EncodeError::StdinWrite`] if the pipe is closed or the child has gone.
    pub fn write_frame(&mut self, frame: &[u8]) -> Result<(), EncodeError> {
        let frames = self.frames_written;
        let stdin = self.stdin.as_mut().ok_or(EncodeError::StdinWrite {
            frames,
            source: std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "ffmpeg stdin was already closed",
            ),
        })?;
        stdin
            .write_all(frame)
            .map_err(|source| EncodeError::StdinWrite { frames, source })?;
        self.frames_written += 1;
        Ok(())
    }

    /// Frames successfully written so far.
    #[must_use]
    pub const fn frames_written(&self) -> u64 {
        self.frames_written
    }

    /// The most recent `-progress` report, or the default before FFmpeg has emitted one.
    #[must_use]
    pub fn progress(&self) -> FfmpegProgress {
        *self.diagnostics.progress.lock().expect("progress mutex")
    }

    /// Whether the child has already exited — a mid-recording death worth surfacing.
    ///
    /// # Errors
    /// Propagates the wait failure.
    pub fn has_exited(&mut self) -> Result<bool, EncodeError> {
        self.child
            .try_wait()
            .map(|status| status.is_some())
            .map_err(|err| EncodeError::Died(err.to_string()))
    }

    /// Closes stdin, waits for FFmpeg to flush and exit, and returns its final progress report.
    ///
    /// # Errors
    /// [`EncodeError::Exited`] with the tail of stderr when FFmpeg reports failure.
    pub fn finish(mut self) -> Result<FfmpegProgress, EncodeError> {
        self.close_stdin();
        self.wait()
    }

    /// Signals end-of-video without waiting for FFmpeg to exit.
    ///
    /// Split out from [`Self::finish`] because the *order* of shutdown across three inputs is
    /// load-bearing, and the two halves belong on opposite sides of it.
    ///
    /// M3 established that a pipe input must be closed before waiting for FFmpeg, or the muxer
    /// never finalizes. Adding the camera overlay produced the mirror image of that deadlock:
    /// the overlay writer blocks in `WriteFile` because FFmpeg — still working through
    /// `h264_mf`'s warm-up backlog — has stopped draining the pipe, and FFmpeg only catches up
    /// once its video stdin is closed. Stopping the camera first therefore waits on FFmpeg,
    /// which is waiting on a stdin close that comes after. Neither ordering works with a single
    /// `finish()`.
    ///
    /// Closing stdin *first* dissolves it: FFmpeg drains its backlog, the blocked writers
    /// unblock on their own, each pipe is then closed normally for a clean EOF, and only then is
    /// the process waited on. Idempotent.
    pub fn close_stdin(&mut self) {
        // Dropping stdin is the shutdown signal: `-i pipe:0` sees EOF, writes the final
        // fragment, and exits cleanly. There is no `q` to send — `-nostdin` is set, and stdin
        // carries video.
        drop(self.stdin.take());
    }

    /// Waits for FFmpeg to exit and reports its final progress.
    ///
    /// Call [`Self::close_stdin`] first, and close every pipe input in between — a muxer reading
    /// a still-open input has no reason to finalize, so this would wait forever.
    ///
    /// # Errors
    /// [`EncodeError::Died`] if the process cannot be waited on, [`EncodeError::Exited`] if it
    /// exited non-zero.
    pub fn wait(mut self) -> Result<FfmpegProgress, EncodeError> {
        let status = self
            .child
            .wait()
            .map_err(|err| EncodeError::Died(err.to_string()))?;

        if let Some(reader) = self.reader.take() {
            // The reader ends on its own at stderr EOF; a panic there is not worth failing an
            // otherwise-good recording over.
            let _ = reader.join();
        }

        if status.success() {
            Ok(self.progress())
        } else {
            Err(EncodeError::Exited {
                code: status.code().unwrap_or(-1),
                stderr_tail: self.diagnostics.tail(),
            })
        }
    }

    /// Replaces a write failure with the reason FFmpeg actually gave, when it gave one.
    ///
    /// A broken pipe is a symptom: it says our end noticed, not why the other end went away.
    /// FFmpeg has almost always already explained itself on stderr — "Unknown encoder",
    /// "No space left on device" — and reporting `BrokenPipe` instead throws that away and
    /// leaves the user with a dead recording and no cause.
    #[must_use]
    pub fn failure_reason(mut self, fallback: EncodeError) -> EncodeError {
        drop(self.stdin.take());
        let status = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        match status {
            Ok(status) if !status.success() => EncodeError::Exited {
                code: status.code().unwrap_or(-1),
                stderr_tail: self.diagnostics.tail(),
            },
            _ => fallback,
        }
    }

    /// Kills FFmpeg without letting it flush. For cancellation only — the tail of the recording
    /// is lost, which is exactly why [`FfmpegProcess::finish`] exists.
    pub fn abort(&mut self) {
        drop(self.stdin.take());
        let _ = self.child.kill();
        let _ = self.child.wait();
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
}

fn drain_stderr(stderr: std::process::ChildStderr, diagnostics: &Diagnostics) {
    let mut parser = ProgressParser::new();
    let reader = BufReader::new(stderr);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if let Some(report) = parser.push_line(&line) {
            *diagnostics.progress.lock().expect("progress mutex") = report;
        } else if !line.trim().is_empty() && !line.contains('=') {
            // Everything with an `=` is either a progress key or a key we do not read; the
            // human-readable warnings are what a failing run needs to explain itself.
            diagnostics.record_line(&line);
        }
    }
}

/// Asks an FFmpeg build which H.264 encoders it actually has, and picks the best (ADR-007).
///
/// Worth doing at startup rather than discovering mid-recording: `h264_mf` is present in every
/// Windows FFmpeg build but *fails to open* on a machine with no Media Foundation encoder, and
/// that failure would otherwise arrive after the user pressed record.
///
/// # Errors
/// [`EncodeError::BinaryNotFound`] if `binary` will not run, or
/// [`EncodeError::EncoderUnavailable`] if the build has neither candidate.
pub fn probe_h264_encoder(binary: &Path) -> Result<H264Encoder, EncodeError> {
    let listing = ffmpeg_output(binary, &["-hide_banner", "-encoders"])?;
    H264Encoder::preference_order()
        .into_iter()
        .find(|encoder| encoder_is_listed(&listing, encoder.ffmpeg_name()))
        .ok_or_else(|| {
            EncodeError::EncoderUnavailable(
                H264Encoder::preference_order()
                    .iter()
                    .map(|encoder| encoder.ffmpeg_name())
                    .collect::<Vec<_>>()
                    .join(" or "),
            )
        })
}

/// Whether `-encoders` listed `name` as an encoder rather than merely mentioning it.
///
/// `-encoders` lines look like ` V....D h264_mf   H264 via MediaFoundation`. A bare substring
/// search would also match the *description* of some other codec, so the name has to sit in the
/// second whitespace-separated column.
fn encoder_is_listed(listing: &str, name: &str) -> bool {
    listing
        .lines()
        .filter_map(|line| line.split_whitespace().nth(1))
        .any(|column| column == name)
}

/// Runs FFmpeg for its text output. Used for capability probes, never on a hot path.
fn ffmpeg_output(binary: &Path, args: &[&str]) -> Result<String, EncodeError> {
    let mut command = Command::new(binary);
    command.args(args).stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command.output().map_err(|err| {
        if err.kind() == std::io::ErrorKind::NotFound {
            EncodeError::BinaryNotFound(PathBuf::from(binary))
        } else {
            EncodeError::Died(err.to_string())
        }
    })?;
    let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
    text.push_str(&String::from_utf8_lossy(&output.stderr));
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_ENCODERS: &str = "\
Encoders:
 V..... = Video
 ------
 V....D libopenh264          OpenH264 H.264 / AVC / MPEG-4 AVC
 V....D h264_mf              H264 via MediaFoundation (codec h264)
 A....D aac                  AAC (Advanced Audio Coding)
";

    #[test]
    fn a_listed_encoder_is_found_in_the_name_column() {
        assert!(encoder_is_listed(SAMPLE_ENCODERS, "h264_mf"));
        assert!(encoder_is_listed(SAMPLE_ENCODERS, "libopenh264"));
    }

    #[test]
    fn a_name_appearing_only_in_a_description_does_not_count() {
        // " V....D h264_mf   H264 via MediaFoundation (codec h264)" mentions `h264`; matching
        // on a substring would report an encoder named `h264` that does not exist.
        assert!(!encoder_is_listed(SAMPLE_ENCODERS, "h264"));
        assert!(!encoder_is_listed(SAMPLE_ENCODERS, "MediaFoundation"));
    }

    #[test]
    fn libx264_is_never_matched_even_if_the_build_has_it() {
        // ADR-007: nothing in vrecnu may select a GPL-only encoder, so it is not a candidate
        // regardless of what the local build happens to contain.
        let gpl_build = format!("{SAMPLE_ENCODERS} V..... libx264   libx264 H.264\n");
        for encoder in H264Encoder::preference_order() {
            assert_ne!(encoder.ffmpeg_name(), "libx264");
        }
        assert!(encoder_is_listed(&gpl_build, "libx264"));
    }

    #[test]
    fn a_missing_binary_is_reported_as_missing_rather_than_dead() {
        // The two produce very different user-facing messages: one says "reinstall", the other
        // says "your recording is gone".
        let err = FfmpegProcess::spawn(
            Path::new("vrecnu-no-such-ffmpeg-binary"),
            &["-version".to_string()],
        )
        .unwrap_err();
        assert!(matches!(err, EncodeError::BinaryNotFound(_)), "{err:?}");
    }

    #[test]
    fn probing_a_missing_binary_reports_it_as_missing() {
        let err = probe_h264_encoder(Path::new("vrecnu-no-such-ffmpeg-binary")).unwrap_err();
        assert!(matches!(err, EncodeError::BinaryNotFound(_)), "{err:?}");
    }

    #[test]
    fn the_stderr_tail_is_bounded() {
        // A five-hour recording must not accumulate a log in memory.
        let diagnostics = Diagnostics::default();
        for index in 0..(STDERR_TAIL_LINES * 3) {
            diagnostics.record_line(&format!("line {index}"));
        }
        let tail = diagnostics.tail();
        assert!(tail.contains(&format!("line {}", STDERR_TAIL_LINES * 3 - 1)));
        assert!(!tail.contains("line 0;"));
    }

    #[test]
    fn progress_lines_update_the_report_and_stay_out_of_the_tail() {
        let diagnostics = Diagnostics::default();
        let mut parser = ProgressParser::new();
        for line in ["frame=90", "progress=continue"] {
            if let Some(report) = parser.push_line(line) {
                *diagnostics.progress.lock().unwrap() = report;
            }
        }
        assert_eq!(diagnostics.progress.lock().unwrap().frame, 90);
        assert!(diagnostics.tail().is_empty());
    }
}
