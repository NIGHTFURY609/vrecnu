/// Failures from the named-pipe server.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PipeError {
    /// The pipe could not be created — most commonly a stale pipe of the same name still open
    /// from a previous session that was never torn down.
    #[error("failed to create named pipe {name}: {source}")]
    Create {
        /// The pipe's short name (without the `\\.\pipe\` prefix).
        name: String,
        /// The underlying OS error.
        #[source]
        source: std::io::Error,
    },
    /// Waiting for a client to connect failed.
    #[error("failed waiting for a client to connect: {0}")]
    Connect(#[source] std::io::Error),
    /// Writing to the connected client failed — most commonly the client (FFmpeg) exited.
    #[error("failed writing to the named pipe: {0}")]
    Write(#[source] std::io::Error),
}
