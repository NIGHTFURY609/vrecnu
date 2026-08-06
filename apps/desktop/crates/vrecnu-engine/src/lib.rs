//! Session orchestration.
//!
//! Owns the state machine and every worker thread. Knows nothing about any shell — that is
//! the whole point, and it is enforced by this crate's `Cargo.toml` rather than by care.
//!
//! Two responsibilities are worth calling out because they are easy to put in the wrong place:
//!
//! - **Event coalescing.** Nothing here is emitted per frame. Audio levels go out at ~15 Hz,
//!   elapsed time at ~4 Hz, stats at ~1 Hz. A per-frame event crossing a WebView IPC boundary
//!   would spend more CPU on notification than on encoding.
//! - **The single error conversion.** Every rich per-crate error funnels through
//!   [`EngineError`] and is turned into a flat `AppError` in exactly one place, so everything
//!   a user ever reads is written once and no shell has to match on forty variants.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod machine;

pub use error::EngineError;
pub use machine::{SessionMachine, TransitionError};
