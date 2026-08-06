//! The floating webcam bubble — "the single biggest reason Phase 2 exists"
//! (ARCHITECTURE.md §5).
//!
//! A **raw Win32 layered window**, not a second Tauri webview and not a second `winit` event
//! loop. Three reasons, in order of weight:
//!
//! 1. **The camera is opened exactly once.** A webview bubble running `getUserMedia` would
//!    need the engine to open the device *again* for the recording overlay. Windows usually
//!    multiplexes that, but virtual and OEM cameras take exclusive control and fail the
//!    second open — after the user has pressed record.
//! 2. **No extra WebView2 process.** ADR-006 already spends most of the RSS budget.
//! 3. **It survives a shell swap untouched.** This crate has no Tauri and no `egui` in it, so
//!    the `egui` shell would keep it as-is. A webview bubble would be thrown away.
//!
//! On the event loop: Tauri v2's loop *is* `tao`, a `winit` fork, and a second `winit`/`tao`
//! loop in one process is unsupported. Win32 message loops are per-thread by design, so a
//! dedicated `RegisterClassW` -> `CreateWindowExW` -> `GetMessageW` thread is ordinary
//! Windows programming and is invisible to Tauri.
//!
//! Presentation is `UpdateLayeredWindow` from a premultiplied-BGRA DIB, which gives true
//! per-pixel alpha — a real circular bubble with a soft shadow and an anti-aliased ring —
//! with no compositor, no swapchain, and no DWM cooperation.

#![forbid(unsafe_code)]
#![warn(missing_docs)]

mod error;
mod geometry;

pub use error::BubbleError;
pub use geometry::{alpha_premultiply, ring_coverage, BubbleLayout};

/// Window styles the bubble needs. Recorded as documentation for the M1 spike; the actual
/// `CreateWindowExW` call arrives with the implementation.
///
/// - `WS_POPUP` — no frame, no title bar.
/// - `WS_EX_LAYERED` — required by `UpdateLayeredWindow`.
/// - `WS_EX_TOPMOST` — stays above other windows.
/// - `WS_EX_TOOLWINDOW` — keeps it out of the taskbar and Alt-Tab.
/// - `WS_EX_NOACTIVATE` — never steals focus from what the user is recording.
/// - `WS_EX_TRANSPARENT` — toggled on for click-through when the bubble is idle.
pub const REQUIRED_EX_STYLES: &[&str] = &[
    "WS_EX_LAYERED",
    "WS_EX_TOPMOST",
    "WS_EX_TOOLWINDOW",
    "WS_EX_NOACTIVATE",
];
