/// Failures specific to the bubble window.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BubbleError {
    /// The window class or window could not be created.
    #[error("could not create the bubble window: {0}")]
    WindowCreation(String),
    /// `UpdateLayeredWindow` rejected the frame.
    #[error("presenting a bubble frame failed: {0}")]
    Present(String),
    /// A frame arrived in a layout the presenter cannot use.
    ///
    /// `UpdateLayeredWindow` requires **premultiplied** BGRA. Straight alpha renders with a
    /// dark fringe around the circle, which looks like a rendering bug rather than a format
    /// mistake — so it is rejected rather than silently drawn.
    #[error("bubble frames must be premultiplied BGRA, got {got}")]
    UnexpectedFormat {
        /// What arrived instead.
        got: String,
    },
    /// The window's message thread stopped.
    #[error("the bubble window thread is no longer running")]
    ThreadGone,
}
