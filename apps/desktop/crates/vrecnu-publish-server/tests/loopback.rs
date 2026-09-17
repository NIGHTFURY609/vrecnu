//! Drives a **real** `PublishServer` over a **real** socket.
//!
//! The unit tests in this crate prove the parsing. They cannot prove that a browser reaching
//! `http://127.0.0.1:47821/` gets the page, that a `Range` request comes back as a 206 with the
//! right bytes, or that a rebound `Host` is refused — and every milestone in this project so far
//! has shipped green unit tests alongside a bug that only appeared when the thing was run.
//!
//! Every test here takes [`PORTS`] first. The five ports in `REGISTERED_PORTS` are a process-wide
//! shared resource, and `cargo test` runs integration tests in parallel by default -- without the
//! lock, one test binds the port another test is asserting about and both fail for reasons that
//! have nothing to do with the code under test. That is not hypothetical; it is how these two
//! first failed.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

use vrecnu_publish_server::{
    Asset, PublishAssets, PublishServer, PublishSignal, RecordingToServe, ServerState,
};

/// Serialises access to the five registered ports.
static PORTS: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The bytes we pretend are a recording. Content is arbitrary; being able to check an exact
/// byte range against it is the point.
fn recording_bytes() -> Vec<u8> {
    (0u32..4096).map(|i| (i % 251) as u8).collect()
}

struct HttpResponse {
    status: u16,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl HttpResponse {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// A deliberately dumb HTTP/1.1 client: one request, one response, connection closed.
///
/// Written by hand rather than pulled in as a dependency so the test exercises exactly the
/// bytes a browser would send — including the `Host` and `Origin` headers, which are what the
/// server's two security checks read.
fn request(port: u16, raw: &str) -> HttpResponse {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect to publish server");
    stream
        .set_read_timeout(Some(Duration::from_secs(10)))
        .unwrap();
    stream.write_all(raw.as_bytes()).unwrap();
    stream.flush().unwrap();

    let mut reader = BufReader::new(stream);
    let mut status_line = String::new();
    reader.read_line(&mut status_line).expect("status line");
    let status: u16 = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|code| code.parse().ok())
        .unwrap_or_else(|| panic!("unparseable status line: {status_line:?}"));

    let mut headers = Vec::new();
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).expect("header line");
        let line = line.trim_end();
        if line.is_empty() {
            break;
        }
        if let Some((key, value)) = line.split_once(':') {
            headers.push((key.trim().to_owned(), value.trim().to_owned()));
        }
    }

    // Content-Length is always present here: nothing this server sends is chunked.
    //
    // A HEAD response still carries the `Content-Length` of the body it *would* have sent --
    // that is the whole reason the page uses HEAD to size the recording -- but sends no body.
    // Reading one anyway is an EOF, which is how this client first failed.
    let length: usize = if raw.starts_with("HEAD ") {
        0
    } else {
        headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("content-length"))
            .and_then(|(_, value)| value.parse().ok())
            .unwrap_or(0)
    };
    let mut body = vec![0u8; length];
    reader.read_exact(&mut body).expect("body");

    HttpResponse {
        status,
        headers,
        body,
    }
}

fn get(port: u16, path: &str) -> HttpResponse {
    request(
        port,
        &format!("GET {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    )
}

fn post(port: u16, path: &str, body: &str) -> HttpResponse {
    request(
        port,
        &format!(
            "POST {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Type: application/json\r\n\
             Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    )
}

#[test]
fn a_publish_session_serves_the_page_the_recording_and_the_result() {
    let _ports = PORTS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("vrecnu-test.mp4");
    let bytes = recording_bytes();
    std::fs::write(&path, &bytes).unwrap();

    let assets = PublishAssets::from_entries(vec![
        Asset {
            path: "index.html".into(),
            mime: "text/html; charset=utf-8".into(),
            bytes: std::borrow::Cow::Borrowed(b"<!doctype html><div id=root></div>"),
        },
        Asset {
            path: "assets/app.js".into(),
            mime: "text/javascript; charset=utf-8".into(),
            bytes: std::borrow::Cow::Borrowed(b"export const x = 1;"),
        },
    ]);

    let mut server = PublishServer::start(
        RecordingToServe {
            path: path.clone(),
            file_name: "vrecnu-test.mp4".into(),
            mime_type: "video/mp4".into(),
            size_bytes: bytes.len() as u64,
            duration_ms: Some(4_000),
            duration_valid: true,
        },
        assets,
    )
    .expect("the publish server should bind a registered port");

    let port = server.port();
    assert!(
        vrecnu_publish_server::REGISTERED_PORTS.contains(&port),
        "must never fall back to an ephemeral port (ADR-002), got {port}"
    );
    assert_eq!(server.state(), ServerState::Serving);

    // The URL carries the token; everything below re-derives it from there rather than being
    // handed it, which is exactly what the page does.
    let token = server
        .url()
        .split_once("?t=")
        .map(|(_, t)| t.to_owned())
        .expect("the publish URL must carry a capability token");
    assert_eq!(token.len(), 64);

    // --- the page itself -----------------------------------------------------------------

    let index = get(port, "/");
    assert_eq!(index.status, 200);
    assert!(index.body.starts_with(b"<!doctype html>"));
    assert_eq!(
        index.header("Content-Type"),
        Some("text/html; charset=utf-8")
    );
    // The one header that must never appear. Same-origin by construction is the whole design.
    assert!(
        index.header("Access-Control-Allow-Origin").is_none(),
        "ADR-002: the loopback server must never emit CORS headers"
    );

    assert_eq!(get(port, "/assets/app.js").status, 200);
    assert_eq!(get(port, "/assets/missing.js").status, 404);

    // --- session metadata ----------------------------------------------------------------

    let session = get(port, &format!("/api/session?t={token}"));
    assert_eq!(session.status, 200);
    let json: serde_json::Value = serde_json::from_slice(&session.body).unwrap();
    assert_eq!(json["fileName"], "vrecnu-test.mp4");
    assert_eq!(json["sizeBytes"], bytes.len());
    assert_eq!(json["durationValid"], true);
    // The page must never learn where the file lives.
    assert!(
        !String::from_utf8_lossy(&session.body).contains("vrecnu-test.mp4\\"),
        "the session document must not leak the on-disk path"
    );

    assert_eq!(
        get(port, "/api/session?t=deadbeef").status,
        403,
        "a wrong token must not read session metadata"
    );
    assert_eq!(get(port, "/api/session").status, 403);

    // --- the recording, whole and ranged -------------------------------------------------

    let head = request(
        port,
        &format!("HEAD /r/{token} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert_eq!(head.status, 200);
    assert_eq!(
        head.header("Content-Length"),
        Some(bytes.len().to_string().as_str()),
        "probeRangeSourceSize() reads exactly this header"
    );
    assert_eq!(head.header("Accept-Ranges"), Some("bytes"));

    let whole = get(port, &format!("/r/{token}"));
    assert_eq!(whole.status, 200);
    assert_eq!(whole.body, bytes);
    assert_eq!(whole.header("Content-Type"), Some("video/mp4"));

    let ranged = request(
        port,
        &format!(
            "GET /r/{token} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nRange: bytes=100-199\r\n\
             Connection: close\r\n\r\n"
        ),
    );
    assert_eq!(ranged.status, 206);
    assert_eq!(ranged.body, bytes[100..=199]);
    assert_eq!(
        ranged.header("Content-Range"),
        Some(format!("bytes 100-199/{}", bytes.len()).as_str())
    );

    let unsatisfiable = request(
        port,
        &format!(
            "GET /r/{token} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nRange: bytes=99999-\r\n\
             Connection: close\r\n\r\n"
        ),
    );
    assert_eq!(
        unsatisfiable.status, 416,
        "an unsatisfiable range served as a 200 would be assembled into a corrupt upload"
    );
    assert_eq!(
        unsatisfiable.header("Content-Range"),
        Some(format!("bytes */{}", bytes.len()).as_str())
    );

    assert_eq!(
        get(port, "/r/deadbeef").status,
        403,
        "the recording must not be readable without the token"
    );

    // --- the two security checks ---------------------------------------------------------

    let rebound = request(
        port,
        &format!("GET / HTTP/1.1\r\nHost: evil.example:{port}\r\nConnection: close\r\n\r\n"),
    );
    assert_eq!(
        rebound.status, 421,
        "DNS rebinding: binding loopback does not defend this, the Host check does"
    );

    let cross_origin = request(
        port,
        &format!(
            "GET /r/{token} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\n\
             Origin: https://evil.example\r\nConnection: close\r\n\r\n"
        ),
    );
    assert_eq!(cross_origin.status, 403);

    // --- reports back from the page ------------------------------------------------------

    let progress = post(
        port,
        "/api/progress",
        &format!(r#"{{"token":"{token}","bytesSent":2048,"total":4096}}"#),
    );
    assert_eq!(progress.status, 204);

    assert_eq!(
        post(
            port,
            "/api/complete",
            r#"{"token":"wrong","fileId":"f","webViewLink":"l"}"#
        )
        .status,
        403
    );
    assert_eq!(
        post(
            port,
            "/api/complete",
            &format!(r#"{{"token":"{token}","fileId":"abc"}}"#)
        )
        .status,
        400,
        "a completion with no link would put an empty string on the clipboard"
    );

    let complete = post(
        port,
        "/api/complete",
        &format!(
            r#"{{"token":"{token}","fileId":"abc123","webViewLink":"https://drive.google.com/file/d/abc123/view"}}"#
        ),
    );
    assert_eq!(complete.status, 204);

    let signals: Vec<PublishSignal> = server.signals().try_iter().collect();
    assert_eq!(
        signals,
        vec![
            PublishSignal::Progress {
                bytes_sent: 2048,
                total: 4096
            },
            PublishSignal::Complete {
                file_id: "abc123".into(),
                web_view_link: "https://drive.google.com/file/d/abc123/view".into(),
            },
        ]
    );

    // A completed session answers 410, not 404: "this publish is over" and "no such page" read
    // differently to the user.
    assert_eq!(get(port, "/").status, 410);

    server.shutdown();
    assert_eq!(server.state(), ServerState::Idle);
    assert!(
        TcpStream::connect(("127.0.0.1", port)).is_err(),
        "a leaked listener makes the *next* publish fail with AllPortsBusy"
    );
}

#[test]
fn dropping_the_server_frees_the_port_for_the_next_publish() {
    let _ports = PORTS.lock().unwrap_or_else(|e| e.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("clip.mp4");
    std::fs::write(&path, b"not really an mp4").unwrap();

    let make = || {
        PublishServer::start(
            RecordingToServe {
                path: path.clone(),
                file_name: "clip.mp4".into(),
                mime_type: "video/mp4".into(),
                size_bytes: 17,
                duration_ms: None,
                duration_valid: false,
            },
            PublishAssets::from_entries(vec![Asset {
                path: "index.html".into(),
                mime: "text/html".into(),
                bytes: std::borrow::Cow::Borrowed(b"<html></html>"),
            }]),
        )
        .expect("bind")
    };

    let first_port = {
        let server = make();
        server.port()
    };
    // Dropped. If `Drop` did not shut the loop down, the next start would skip this port.
    let second = make();
    assert_eq!(
        second.port(),
        first_port,
        "the same registered port must be reusable immediately after a session ends"
    );
}
