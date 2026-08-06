//! Headless vrecnu.
//!
//! **This binary is the swappability guarantee** (ADR-006). It drives the same engine the
//! Tauri shell drives, with zero Tauri anywhere in its dependency graph. CI builds it on
//! every commit, so the day someone reaches for `tauri::path` inside the engine, this stops
//! linking and the build goes red — rather than the escape hatch quietly closing and nobody
//! finding out until they try to use it.
//!
//! It is also genuinely useful: capture, encode and publish can be exercised end to end
//! without a window, which is what makes M2–M5 testable before any UI exists.

#![forbid(unsafe_code)]

use std::process::ExitCode;

mod commands;

fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_env("VRECNU_LOG")
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    match commands::run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::FAILURE
        }
    }
}
