//! A minimal Win32 named-pipe server.
//!
//! One job: hand FFmpeg's second `-i` input mixed PCM audio without an intermediate file (ADR-
//! 007 — audio reaches FFmpeg over a Windows named pipe). Windows has no server-side named-pipe
//! API in `std`, so this is unavoidably raw Win32 FFI — confined to this one crate, the same way
//! `vrecnu-bubble-win32` confines its own unavoidable `unsafe` to a single file.

#![warn(missing_docs)]
#![deny(unsafe_op_in_unsafe_fn)]

#[cfg(windows)]
mod pipe;

mod error;

pub use error::PipeError;
#[cfg(windows)]
pub use pipe::{pipe_path, NamedPipeServer};
