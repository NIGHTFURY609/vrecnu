//! The server loop. Everything the rest of this crate tests as pure logic, actually running.
//!
//! Shape of a publish session, start to finish:
//!
//! ```text
//! begin  ─►  bind 47821..47825  ─►  serve index.html + /r/<token>  ─►  POST /api/complete
//!            (SO_EXCLUSIVEADDRUSE)   (same origin, no CORS)             ─► signal ─► shutdown
//! ```
//!
//! The server exists only for the length of one publish. It is not started at app launch, it
//! serves exactly one recording, and it dies when the page reports a result or the session
//! times out.

use std::io::{Read, Seek, SeekFrom};
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpListener};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{unbounded, Receiver, Sender};
use parking_lot::Mutex;
use tiny_http::{Header, Request, Response, StatusCode};

use crate::assets::{asset_key_for, PublishAssets};
use crate::range::parse_range_header;
use crate::{
    is_acceptable_host, is_acceptable_origin, CapabilityToken, PublishError, ServerState,
    ABSOLUTE_TIMEOUT_SECS, IDLE_TIMEOUT_SECS, REGISTERED_PORTS,
};

/// How long a POST body may be before it is refused.
///
/// The three POST endpoints carry a token, a file id and a URL. Anything larger is either a
/// bug or an attempt to make the server hold a request-sized allocation.
const MAX_BODY_BYTES: u64 = 64 * 1024;

/// How often the loop wakes to re-check shutdown and the timeouts.
const POLL_INTERVAL: Duration = Duration::from_millis(200);

/// The recording a publish session is about.
#[derive(Debug, Clone)]
pub struct RecordingToServe {
    /// Absolute path on disk. Read-only, and never revealed to the page.
    pub path: PathBuf,
    /// The name to suggest to Drive.
    pub file_name: String,
    /// MIME type for the Drive upload metadata.
    pub mime_type: String,
    /// Size in bytes, as measured when the session began.
    pub size_bytes: u64,
    /// Duration, when the muxer wrote one.
    pub duration_ms: Option<u64>,
    /// Whether that duration is trustworthy. A `false` here makes the page warn that Drive's
    /// preview player may not seek.
    pub duration_valid: bool,
}

/// What the publish page reported back over the loopback origin.
///
/// This is the *only* channel the result arrives on. ADR-006 forbids routing it through shell
/// IPC, because that would put a shell-specific branch in `packages/publish` and break the
/// page's ability to run unchanged in a plain browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PublishSignal {
    /// Upload progress.
    Progress {
        /// Bytes Drive has acknowledged.
        bytes_sent: u64,
        /// Total bytes.
        total: u64,
    },
    /// The upload finished and a share link exists.
    Complete {
        /// Drive file id.
        file_id: String,
        /// Drive's `webViewLink`.
        web_view_link: String,
    },
    /// The page reported a failure.
    Failed {
        /// What it said.
        message: String,
    },
    /// Nobody finished the publish in time and the server tore itself down.
    TimedOut {
        /// Which limit ran out.
        after: Duration,
    },
}

/// A running publish session.
///
/// Dropping this shuts the server down. That is deliberate: a leaked listener on a registered
/// port makes the *next* publish fail with `AllPortsBusy`, which reads like a user problem and
/// is not one.
pub struct PublishServer {
    port: u16,
    url: String,
    shutdown: Arc<AtomicBool>,
    /// `Option` because `shutdown` has to *drop* it. Stopping the accept loop is not the same
    /// as closing the listener: while any `Arc` survives, the port stays bound and the next
    /// publish skips past it -- eventually to `AllPortsBusy`, which reads as a user problem.
    server: Option<Arc<tiny_http::Server>>,
    state: Arc<Mutex<ServerState>>,
    signals: Receiver<PublishSignal>,
    worker: Option<JoinHandle<()>>,
}

impl std::fmt::Debug for PublishServer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never `{self.url}` -- it carries the capability token.
        f.debug_struct("PublishServer")
            .field("port", &self.port)
            .field("state", &*self.state.lock())
            .finish_non_exhaustive()
    }
}

impl PublishServer {
    /// Binds a registered port and starts serving one recording.
    ///
    /// # Errors
    ///
    /// [`PublishError::AllPortsBusy`] if all five registered ports are taken — never an
    /// ephemeral port, which Google would reject as an unregistered origin and report as its
    /// own error screen rather than one vrecnu can explain (ADR-002).
    pub fn start(recording: RecordingToServe, assets: PublishAssets) -> Result<Self, PublishError> {
        if assets.get("index.html").is_none() {
            return Err(PublishError::AssetsMissing);
        }
        if !recording.path.is_file() {
            return Err(PublishError::RecordingMissing(recording.path));
        }

        let (listener, port) = bind_a_registered_port()?;
        let server = tiny_http::Server::from_listener(listener, None).map_err(|source| {
            PublishError::Bind {
                port,
                source: std::io::Error::other(source.to_string()),
            }
        })?;
        let server = Arc::new(server);

        let token = CapabilityToken::random();
        let url = format!("http://127.0.0.1:{port}/?t={}", token.as_str());

        let shutdown = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(ServerState::Serving));
        let (signal_tx, signals) = unbounded();

        let ctx = Ctx {
            port,
            token,
            recording,
            assets,
            state: Arc::clone(&state),
            shutdown: Arc::clone(&shutdown),
            signals: signal_tx,
        };
        let loop_server = Arc::clone(&server);
        let worker = std::thread::Builder::new()
            .name("vrecnu-publish".into())
            .spawn(move || serve(&loop_server, &ctx))
            .map_err(|source| PublishError::Bind { port, source })?;

        tracing::info!(port, "publish server listening");
        Ok(Self {
            port,
            url,
            shutdown,
            server: Some(server),
            state,
            signals,
            worker: Some(worker),
        })
    }

    /// Where to point a browser. **Contains the capability token — never log it.**
    #[must_use]
    pub fn url(&self) -> &str {
        &self.url
    }

    /// The port that was actually bound.
    #[must_use]
    pub fn port(&self) -> u16 {
        self.port
    }

    /// What the page has reported so far.
    #[must_use]
    pub fn signals(&self) -> &Receiver<PublishSignal> {
        &self.signals
    }

    /// Current lifecycle state.
    #[must_use]
    pub fn state(&self) -> ServerState {
        *self.state.lock()
    }

    /// Stops serving and waits for the loop to exit.
    ///
    /// Idempotent, and also what `Drop` calls.
    pub fn shutdown(&mut self) {
        if self.shutdown.swap(true, Ordering::SeqCst) {
            return;
        }
        *self.state.lock() = ServerState::Draining;
        // Wakes a `recv_timeout` that is mid-wait, so shutdown is prompt rather than up to
        // one poll interval late.
        if let Some(server) = &self.server {
            server.unblock();
        }
        // Join first: the worker holds the other `Arc`, so dropping ours before it exits would
        // leave the listener bound anyway.
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
        self.server = None;
        wait_for_port_release(self.port);
        *self.state.lock() = ServerState::Idle;
        tracing::info!(port = self.port, "publish server stopped");
    }
}

impl Drop for PublishServer {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Everything the request handlers need. Immutable except the two shared flags.
struct Ctx {
    port: u16,
    token: CapabilityToken,
    recording: RecordingToServe,
    assets: PublishAssets,
    state: Arc<Mutex<ServerState>>,
    shutdown: Arc<AtomicBool>,
    signals: Sender<PublishSignal>,
}

fn serve(server: &tiny_http::Server, ctx: &Ctx) {
    let started = Instant::now();
    let mut last_activity = Instant::now();

    while !ctx.shutdown.load(Ordering::SeqCst) {
        match server.recv_timeout(POLL_INTERVAL) {
            Ok(Some(request)) => {
                last_activity = Instant::now();
                handle(request, ctx);
            }
            Ok(None) => {}
            Err(err) => {
                tracing::warn!(error = %err, "publish server accept failed");
                break;
            }
        }

        // Two separate limits on purpose. Idle catches the user who opened the page and
        // wandered off; absolute catches a genuinely enormous upload that keeps the socket
        // warm forever. Neither may leave a listener on a registered port.
        let idle = last_activity.elapsed();
        let total = started.elapsed();
        if idle >= Duration::from_secs(IDLE_TIMEOUT_SECS) {
            let _ = ctx.signals.send(PublishSignal::TimedOut { after: idle });
            break;
        }
        if total >= Duration::from_secs(ABSOLUTE_TIMEOUT_SECS) {
            let _ = ctx.signals.send(PublishSignal::TimedOut { after: total });
            break;
        }
    }

    *ctx.state.lock() = ServerState::Draining;
}

fn handle(mut request: Request, ctx: &Ctx) {
    let (path, query) = split_url(request.url());
    let path = path.to_owned();
    let query = query.to_owned();
    let method = request.method().clone();

    // Draining answers 410 rather than 404: "this publish session is over" and "no such page"
    // are different things, and the page shows different text for each.
    if *ctx.state.lock() == ServerState::Draining {
        respond(request, plain(410, "This publish session has ended."));
        return;
    }

    // Host before anything else. This is the DNS-rebinding defence -- binding 127.0.0.1 does
    // not provide it, because a hostile page can point its own name at 127.0.0.1 and then
    // treat our responses as same-origin to *itself*.
    let host = header_value(&request, "Host").unwrap_or_default();
    if !is_acceptable_host(&host, ctx.port) {
        tracing::warn!(%host, "rejected a request with a foreign Host header");
        respond(request, plain(421, "Misdirected request."));
        return;
    }
    let origin = header_value(&request, "Origin");
    if !is_acceptable_origin(origin.as_deref(), ctx.port) {
        tracing::warn!(origin = ?origin, "rejected a cross-origin request");
        respond(request, plain(403, "Forbidden."));
        return;
    }

    match (method, path.as_str()) {
        (tiny_http::Method::Get | tiny_http::Method::Head, "/api/session") => {
            if !ctx.token.matches(&token_from_query(&query)) {
                respond(request, plain(403, "Forbidden."));
                return;
            }
            respond(request, session_json(ctx));
        }
        (tiny_http::Method::Get | tiny_http::Method::Head, _) if path.starts_with("/r/") => {
            let presented = percent_decode(&path["/r/".len()..]);
            if !ctx.token.matches(&presented) {
                respond(request, plain(403, "Forbidden."));
                return;
            }
            serve_recording(request, ctx);
        }
        (tiny_http::Method::Post, "/api/progress" | "/api/complete" | "/api/error") => {
            let body = match read_body(&mut request) {
                Ok(body) => body,
                Err(response) => {
                    respond(request, response);
                    return;
                }
            };
            respond(request, handle_report(&path, &body, ctx));
        }
        (tiny_http::Method::Get | tiny_http::Method::Head, _) => {
            let Some(key) = asset_key_for(&path) else {
                respond(request, plain(404, "Not found."));
                return;
            };
            let Some(asset) = ctx.assets.get(&key) else {
                respond(request, plain(404, "Not found."));
                return;
            };
            let response = Response::from_data(asset.bytes.as_ref())
                .with_header(content_type(&asset.mime))
                .with_header(no_store())
                .with_header(nosniff());
            respond(request, response);
        }
        _ => respond(request, plain(405, "Method not allowed.")),
    }
}

/// `GET /api/session` — what the page needs to describe the recording before uploading it.
///
/// Note what is *not* here: the path on disk. The page never learns where the file lives, so
/// a bug in it cannot leak the user's directory layout into a Drive filename or an error
/// string that gets pasted into an issue.
fn session_json(ctx: &Ctx) -> Response<std::io::Cursor<Vec<u8>>> {
    let body = serde_json::json!({
        "fileName": ctx.recording.file_name,
        "mimeType": ctx.recording.mime_type,
        "sizeBytes": ctx.recording.size_bytes,
        "durationMs": ctx.recording.duration_ms,
        "durationValid": ctx.recording.duration_valid,
    });
    Response::from_data(body.to_string().into_bytes())
        .with_header(content_type("application/json; charset=utf-8"))
        .with_header(no_store())
        .with_header(nosniff())
}

/// `GET /r/<token>` — the recording itself, with Range support.
///
/// Range is not an optimisation here. The page reads the file through a ranged `ByteSource`
/// precisely so a 500 MB recording never becomes a 500 MB `Blob` in the web view, which is the
/// same memory failure ADR-003 exists to prevent on the capture side.
fn serve_recording(request: Request, ctx: &Ctx) {
    let mut file = match std::fs::File::open(&ctx.recording.path) {
        Ok(file) => file,
        Err(err) => {
            tracing::error!(error = %err, "the recording vanished mid-publish");
            respond(request, plain(410, "The recording is no longer available."));
            return;
        }
    };
    // The size is re-read rather than trusted from `RecordingToServe`: an edit may have
    // rewritten the file between `begin_publish` and the page's first byte, and a stale
    // Content-Length produces a truncated upload that Drive accepts and the user does not
    // notice until they play it.
    let total = match file.metadata() {
        Ok(meta) => meta.len(),
        Err(err) => {
            tracing::error!(error = %err, "could not stat the recording");
            respond(request, plain(410, "The recording is no longer available."));
            return;
        }
    };

    let range_header = header_value(&request, "Range");
    let spec = match range_header
        .as_deref()
        .and_then(|h| parse_range_header(h, total))
    {
        Some(Ok(spec)) => Some(spec),
        // An unsatisfiable range must be 416 with the total, not a silent whole-file 200 --
        // the page would treat the 200 as the requested chunk and assemble garbage.
        Some(Err(())) => {
            let response = plain(416, "Requested range not satisfiable.")
                .with_header(header("Content-Range", &format!("bytes */{total}")));
            respond(request, response);
            return;
        }
        // Unparseable or multi-range: serve the whole file, which is what RFC 9110 allows.
        None => None,
    };

    let (status, start, length) = match spec {
        Some(spec) => (206u16, spec.start, spec.len()),
        None => (200u16, 0, total),
    };

    if file.seek(SeekFrom::Start(start)).is_err() {
        respond(request, plain(500, "Could not read the recording."));
        return;
    }

    let mut response = Response::new(
        StatusCode(status),
        vec![
            content_type(&ctx.recording.mime_type),
            no_store(),
            nosniff(),
            header("Accept-Ranges", "bytes"),
        ],
        file.take(length),
        // `usize` on a 32-bit host cannot hold a >4 GB recording. Reporting no length is far
        // better than reporting a wrapped one: the client falls back to reading to EOF.
        usize::try_from(length).ok(),
        None,
    );
    if let Some(spec) = spec {
        response.add_header(header("Content-Range", &spec.content_range(total)));
    }
    respond(request, response);
}

/// The three `POST /api/*` endpoints.
fn handle_report(path: &str, body: &str, ctx: &Ctx) -> Response<std::io::Cursor<Vec<u8>>> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return plain(400, "Malformed report.");
    };
    let presented = value.get("token").and_then(|t| t.as_str()).unwrap_or("");
    if !ctx.token.matches(presented) {
        return plain(403, "Forbidden.");
    }

    let signal = match path {
        "/api/progress" => PublishSignal::Progress {
            bytes_sent: value.get("bytesSent").and_then(|v| v.as_u64()).unwrap_or(0),
            total: value
                .get("total")
                .and_then(|v| v.as_u64())
                .unwrap_or(ctx.recording.size_bytes),
        },
        "/api/complete" => {
            let file_id = value.get("fileId").and_then(|v| v.as_str()).unwrap_or("");
            let link = value
                .get("webViewLink")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            // A "success" with no link is a failure with better manners. Accepting it would
            // put an empty string on the user's clipboard and call it done.
            if file_id.is_empty() || link.is_empty() {
                return plain(400, "The report was missing a file id or link.");
            }
            PublishSignal::Complete {
                file_id: file_id.to_owned(),
                web_view_link: link.to_owned(),
            }
        }
        "/api/error" => PublishSignal::Failed {
            message: value
                .get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("The publish page reported an unspecified failure.")
                .to_owned(),
        },
        _ => return plain(404, "Not found."),
    };

    let terminal = matches!(
        signal,
        PublishSignal::Complete { .. } | PublishSignal::Failed { .. }
    );
    let _ = ctx.signals.send(signal);
    if terminal {
        // Drain rather than stop outright: the page is still on screen and will fetch its own
        // assets for a moment longer. `PublishServer::shutdown` closes the listener once the
        // engine has acted on the signal.
        *ctx.state.lock() = ServerState::Draining;
    }
    plain(204, "")
}

// --- plumbing ---------------------------------------------------------------------------

/// Blocks until the OS agrees `port` is free again, or gives up after [`RELEASE_GRACE`].
///
/// Dropping the listener is not the same as the port being bindable: tiny_http's own accept
/// thread unwinds asynchronously, and its shutdown self-connection lingers a moment longer. So
/// `shutdown()` returning was only *probably* enough, and the observable consequence was the
/// next publish skipping a free port -- or, with all five recently used, `AllPortsBusy` on a
/// machine where nothing is actually listening.
///
/// Confirmed by binding it, which is the only answer the OS will actually give.
fn wait_for_port_release(port: u16) {
    let deadline = Instant::now() + RELEASE_GRACE;
    loop {
        match bind_exclusive(port) {
            Ok(listener) => {
                drop(listener);
                return;
            }
            Err(_) if Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(10));
            }
            // Not worth failing over: the caller has already stopped serving, and the next
            // `begin_publish` will simply move to the next registered port.
            Err(err) => {
                tracing::debug!(port, error = %err, "publish port was still held at shutdown");
                return;
            }
        }
    }
}

/// How long [`PublishServer::shutdown`] will wait for the OS to release the port.
///
/// Short enough not to stall a shell's UI thread; long enough for a loopback listener, which is
/// released in milliseconds and never enters `TIME_WAIT` (only accepted sockets do).
const RELEASE_GRACE: Duration = Duration::from_millis(1500);

/// Binds the first free registered port, with hijack protection.
fn bind_a_registered_port() -> Result<(TcpListener, u16), PublishError> {
    let mut last_hard_error = None;
    for port in REGISTERED_PORTS {
        match bind_exclusive(port) {
            Ok(listener) => return Ok((listener, port)),
            Err(err) if is_port_taken(&err) => {
                tracing::debug!(port, "publish port is taken, trying the next");
            }
            Err(source) => last_hard_error = Some(PublishError::Bind { port, source }),
        }
    }
    Err(last_hard_error.unwrap_or(PublishError::AllPortsBusy(REGISTERED_PORTS)))
}

fn is_port_taken(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::AddrInUse | std::io::ErrorKind::PermissionDenied
    )
}

/// Creates the listening socket in four steps so `SO_EXCLUSIVEADDRUSE` can be set between
/// socket creation and bind.
///
/// `TcpListener::bind` does socket+bind+listen in one call and leaves no window for it. Why it
/// matters: on Windows, *without* this option another local process can bind the same
/// `127.0.0.1:47821` using `SO_REUSEADDR` and steal connections. That would let unprivileged
/// local malware serve its own page on an origin the user's Google account has authorised —
/// exactly the origin ADR-002 registers. `SO_REUSEADDR` is the wrong option here and would
/// *create* the hole rather than close it.
fn bind_exclusive(port: u16) -> std::io::Result<TcpListener> {
    use socket2::{Domain, Socket, Type};

    let address = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));
    let socket = Socket::new(Domain::IPV4, Type::STREAM, None)?;
    set_exclusive_address_use(&socket)?;
    socket.bind(&address.into())?;
    socket.listen(16)?;
    Ok(socket.into())
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn set_exclusive_address_use(socket: &socket2::Socket) -> std::io::Result<()> {
    use std::os::windows::io::AsRawSocket;

    use windows::Win32::Networking::WinSock::{
        setsockopt, SOCKET, SOCKET_ERROR, SOL_SOCKET, SO_EXCLUSIVEADDRUSE,
    };

    let enable: i32 = 1;
    // SAFETY: `socket` outlives the call, `SO_EXCLUSIVEADDRUSE` takes a BOOL-shaped `i32`, and
    // the slice we hand over is exactly `size_of::<i32>()` bytes of initialised memory owned by
    // this frame. socket2 0.5 exposes no safe wrapper for this option; this is the crate's only
    // `unsafe`, which is why `lib.rs` says `deny` rather than `forbid`.
    let result = unsafe {
        setsockopt(
            SOCKET(socket.as_raw_socket() as usize),
            SOL_SOCKET,
            SO_EXCLUSIVEADDRUSE,
            Some(&enable.to_ne_bytes()),
        )
    };
    if result == SOCKET_ERROR {
        return Err(std::io::Error::last_os_error());
    }
    Ok(())
}

/// Non-Windows builds get a plain socket.
///
/// The desktop app is Windows-only today; this exists so the crate's logic and tests compile
/// on a contributor's Linux machine rather than being unreviewable there.
#[cfg(not(windows))]
fn set_exclusive_address_use(_socket: &socket2::Socket) -> std::io::Result<()> {
    Ok(())
}

/// `name` is `&'static str` because `HeaderField::equiv` demands one -- every caller passes a
/// literal anyway.
fn header_value(request: &Request, name: &'static str) -> Option<String> {
    request
        .headers()
        .iter()
        .find(|h| h.field.equiv(name))
        .map(|h| h.value.as_str().to_owned())
}

fn split_url(url: &str) -> (&str, &str) {
    url.split_once('?').unwrap_or((url, ""))
}

/// Pulls `t=` out of a query string.
fn token_from_query(query: &str) -> String {
    query
        .split('&')
        .filter_map(|pair| pair.split_once('='))
        .find(|(key, _)| *key == "t")
        .map(|(_, value)| percent_decode(value))
        .unwrap_or_default()
}

/// Minimal percent-decoding — enough for a hex token, which is all that reaches it.
fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn read_body(request: &mut Request) -> Result<String, Response<std::io::Cursor<Vec<u8>>>> {
    if request
        .body_length()
        .is_some_and(|len| len as u64 > MAX_BODY_BYTES)
    {
        return Err(plain(413, "Report too large."));
    }
    let mut body = String::new();
    // `take` even when the length header lied: a chunked body has no declared length at all.
    request
        .as_reader()
        .take(MAX_BODY_BYTES)
        .read_to_string(&mut body)
        .map_err(|_| plain(400, "Could not read the report."))?;
    Ok(body)
}

fn plain(status: u16, message: &str) -> Response<std::io::Cursor<Vec<u8>>> {
    Response::from_data(message.as_bytes().to_vec())
        .with_status_code(StatusCode(status))
        .with_header(content_type("text/plain; charset=utf-8"))
        .with_header(no_store())
        .with_header(nosniff())
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes())
        .expect("header names and values here are compile-time constants or ASCII")
}

fn content_type(mime: &str) -> Header {
    header("Content-Type", mime)
}

/// The recording and the session metadata must never reach a disk cache. Note there is
/// deliberately no `Access-Control-Allow-Origin` anywhere in this file (ADR-002).
fn no_store() -> Header {
    header("Cache-Control", "no-store")
}

fn nosniff() -> Header {
    header("X-Content-Type-Options", "nosniff")
}

fn respond<R: Read>(request: Request, response: Response<R>) {
    if let Err(err) = request.respond(response) {
        // Normal: the page navigated away mid-transfer, or the web view was closed.
        tracing::debug!(error = %err, "publish response was not delivered");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_splits_into_path_and_query() {
        assert_eq!(split_url("/api/session?t=abc"), ("/api/session", "t=abc"));
        assert_eq!(split_url("/"), ("/", ""));
    }

    #[test]
    fn the_token_is_read_out_of_the_query_string() {
        assert_eq!(token_from_query("t=abc123"), "abc123");
        assert_eq!(token_from_query("x=1&t=abc123&y=2"), "abc123");
        assert_eq!(token_from_query(""), "");
        assert_eq!(token_from_query("tt=abc"), "");
    }

    #[test]
    fn a_percent_encoded_token_decodes_back() {
        assert_eq!(percent_decode("abc%2Ddef"), "abc-def");
        assert_eq!(percent_decode("plain"), "plain");
        // A trailing stray `%` must not panic or eat the string.
        assert_eq!(percent_decode("abc%"), "abc%");
    }

    #[test]
    fn a_missing_recording_is_refused_before_a_port_is_bound() {
        // Binding first would burn a registered port to discover something we can check for
        // free, and `AllPortsBusy` is a far more confusing error than "the file is gone".
        let err = PublishServer::start(
            RecordingToServe {
                path: PathBuf::from("Z:/does/not/exist.mp4"),
                file_name: "x.mp4".into(),
                mime_type: "video/mp4".into(),
                size_bytes: 0,
                duration_ms: None,
                duration_valid: false,
            },
            PublishAssets::from_entries(vec![crate::assets::Asset {
                path: "index.html".into(),
                mime: "text/html".into(),
                bytes: std::borrow::Cow::Borrowed(b"<html></html>"),
            }]),
        )
        .unwrap_err();
        assert!(matches!(err, PublishError::RecordingMissing(_)));
    }

    #[test]
    fn a_build_without_the_publish_page_refuses_to_serve() {
        let err = PublishServer::start(
            RecordingToServe {
                path: PathBuf::from("Z:/does/not/exist.mp4"),
                file_name: "x.mp4".into(),
                mime_type: "video/mp4".into(),
                size_bytes: 0,
                duration_ms: None,
                duration_valid: false,
            },
            PublishAssets::from_entries(vec![]),
        )
        .unwrap_err();
        // Checked before the missing recording: a build with no page cannot publish anything,
        // so reporting the file is the less useful of the two truths.
        assert!(matches!(err, PublishError::AssetsMissing));
    }
}
