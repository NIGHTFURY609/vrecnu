use std::path::PathBuf;
use std::time::Duration;

/// Failures specific to the FFmpeg sidecar.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EncodeError {
    /// The bundled binary is missing from the install directory.
    #[error("ffmpeg.exe was not found at {0}; reinstalling vrecnu should restore it")]
    BinaryNotFound(PathBuf),
    /// FFmpeg ran and exited with a failure status.
    #[error("ffmpeg exited with code {code}: {stderr_tail}")]
    Exited {
        /// Process exit code.
        code: i32,
        /// Last few lines of stderr, which is where FFmpeg explains itself.
        stderr_tail: String,
    },
    /// FFmpeg disappeared without an exit status we could read.
    #[error("ffmpeg terminated unexpectedly ({0})")]
    Died(String),
    /// A named pipe could not be created or connected.
    #[error("named pipe {name} failed: {source}")]
    Pipe {
        /// Pipe name.
        name: String,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The requested encoder is not present in this FFmpeg build.
    #[error("encoder {0} is unavailable in the bundled ffmpeg build")]
    EncoderUnavailable(String),
    /// Writing a frame to FFmpeg's stdin failed.
    #[error("writing to ffmpeg failed after {frames} frames: {source}")]
    StdinWrite {
        /// Frames successfully written before the failure.
        frames: u64,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// A render exceeded its time budget.
    #[error("render timed out after {0:?}")]
    Timeout(Duration),
}
