//! The loopback publish server (ADR-002).
//!
//! The desktop app serves the bundled publish page from `http://127.0.0.1:<port>` and serves
//! **the one current recording from that same origin**. Same-origin by construction is the
//! entire design: the page fetches the video with no CORS grant, and the capability token
//! never has to survive a cross-origin request. ADR-002 puts it as "C removes a security
//! control by removing the need for it."
//!
//! Constraints that are not negotiable:
//!
//! - Bind `127.0.0.1` only. Never a network-facing listener.
//! - Serve exactly two things: the embedded page assets, and the current recording. Never a
//!   directory. "Serve the recordings folder on localhost" is explicitly forbidden.
//! - Try the five registered ports in ascending order and **fail loudly if all are busy**. An
//!   ephemeral port is not registered with Google and produces a confusing Google error
//!   screen instead of a vrecnu one.
//! - Never emit `Access-Control-Allow-Origin`.
//!
//! `deny` rather than `forbid` below: the crate has exactly one `unsafe` call, `setsockopt` for
//! `SO_EXCLUSIVEADDRUSE` in `server::bind_exclusive`, which neither std nor socket2 exposes
//! safely and which is what stops another local process hijacking a registered port.

#![deny(unsafe_code)]
#![warn(missing_docs)]

mod assets;
mod error;
mod range;
mod server;
mod token;

pub use assets::{asset_key_for, Asset, PublishAssets};
pub use error::PublishError;
pub use range::{parse_range_header, RangeSpec};
pub use server::{PublishServer, PublishSignal, RecordingToServe};
pub use token::CapabilityToken;

/// The five ports registered as Authorized JavaScript Origins on the OAuth client.
///
/// Changing this list means re-registering every origin in Google Cloud Console, and in every
/// bring-your-own-credentials user's project too. It is effectively immutable.
pub const REGISTERED_PORTS: [u16; 5] = [47821, 47822, 47823, 47824, 47825];

/// Hosts the server will answer to.
///
/// **This is the DNS-rebinding defence, and "loopback-only" does not provide it.** A hostile
/// page can point `evil.example` at `127.0.0.1`; the browser then treats our responses as
/// same-origin *to evil.example*. Binding to loopback does nothing about that — checking the
/// `Host` header does.
#[must_use]
pub fn is_acceptable_host(host: &str, port: u16) -> bool {
    host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
}

/// Whether an `Origin` header is one of ours.
///
/// A same-origin `GET` sends no `Origin` at all, so absence is fine; a cross-origin request
/// always sends one, and any value that is not our own loopback origin is rejected.
#[must_use]
pub fn is_acceptable_origin(origin: Option<&str>, port: u16) -> bool {
    match origin {
        None => true,
        Some(value) => value == format!("http://127.0.0.1:{port}"),
    }
}

/// Lifecycle of the publish server.
///
/// The server **does not exist** until a publish begins — it is not started at app launch.
/// That keeps it off the idle CPU/RSS budget and off the attack surface for the 99% of the
/// time nobody is publishing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServerState {
    /// Not listening.
    Idle,
    /// Choosing a port.
    Binding,
    /// Serving one recording to one publish session.
    Serving,
    /// Shutting down; every handler now answers `410 Gone`.
    Draining,
}

/// How long a publish session may sit idle before it is torn down.
pub const IDLE_TIMEOUT_SECS: u64 = 30 * 60;
/// Hard ceiling on a publish session regardless of activity.
pub const ABSOLUTE_TIMEOUT_SECS: u64 = 4 * 60 * 60;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_registered_ports_are_exactly_adr_002s_list_in_order() {
        // Re-registering these in Google Cloud is a manual step for us and for every BYO
        // user, so a silent change here would break publishing for everyone.
        assert_eq!(REGISTERED_PORTS, [47821, 47822, 47823, 47824, 47825]);
    }

    #[test]
    fn our_own_loopback_hosts_are_accepted() {
        assert!(is_acceptable_host("127.0.0.1:47821", 47821));
        assert!(is_acceptable_host("localhost:47821", 47821));
    }

    #[test]
    fn a_rebound_hostname_is_rejected() {
        // The attack loopback-only binding does not stop.
        assert!(!is_acceptable_host("evil.example:47821", 47821));
        assert!(!is_acceptable_host("evil.example", 47821));
    }

    #[test]
    fn a_host_naming_a_different_port_is_rejected() {
        assert!(!is_acceptable_host("127.0.0.1:47822", 47821));
    }

    #[test]
    fn a_missing_origin_is_fine_because_same_origin_gets_omit_it() {
        assert!(is_acceptable_origin(None, 47821));
    }

    #[test]
    fn a_foreign_origin_is_rejected() {
        assert!(!is_acceptable_origin(Some("https://evil.example"), 47821));
        assert!(!is_acceptable_origin(Some("http://localhost:47821"), 47821));
    }

    #[test]
    fn our_own_origin_is_accepted() {
        assert!(is_acceptable_origin(Some("http://127.0.0.1:47821"), 47821));
    }
}
