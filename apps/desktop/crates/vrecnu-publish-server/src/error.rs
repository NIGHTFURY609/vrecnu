use std::path::PathBuf;
use std::time::Duration;

/// Failures specific to the loopback publish server.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PublishError {
    /// Every registered port is occupied.
    ///
    /// ADR-002 forbids falling back to an ephemeral port: it is not registered as an
    /// Authorized JavaScript Origin, so the user would get a confusing Google error screen
    /// instead of an actionable vrecnu one.
    #[error("all publish ports are in use: {0:?}")]
    AllPortsBusy([u16; 5]),
    /// Binding a specific port failed for a reason other than it being taken.
    #[error("could not bind 127.0.0.1:{port}: {source}")]
    Bind {
        /// The port attempted.
        port: u16,
        /// Underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The build did not embed `packages/publish/dist`.
    #[error("the publish page assets were not embedded in this build")]
    AssetsMissing,
    /// A request presented a token that does not match.
    #[error("invalid or expired capability token")]
    BadToken,
    /// A publish session is already serving.
    ///
    /// One recording, one session, one token — a `Serving -> Serving` transition would leave
    /// the previous token live against a file nobody is publishing any more.
    #[error("a publish session is already active")]
    AlreadyActive,
    /// The session outlived its idle or absolute timeout.
    #[error("the publish session timed out after {0:?}")]
    TimedOut(Duration),
    /// The publish page reported a failure of its own.
    #[error("the publish page reported: {0}")]
    PageReported(String),
    /// The recording disappeared while it was being served.
    #[error("the recording at {0} is no longer available")]
    RecordingMissing(PathBuf),
}
