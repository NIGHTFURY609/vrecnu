use windows::core::HSTRING;
use windows::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{FlushFileBuffers, WriteFile, PIPE_ACCESS_OUTBOUND};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, PIPE_READMODE_BYTE, PIPE_TYPE_BYTE,
    PIPE_WAIT,
};

use crate::PipeError;

/// Windows error code for "a client connected between `CreateNamedPipeW` and `ConnectNamedPipe`"
/// — a race, not a failure. `ConnectNamedPipe` reports it as a `FALSE` return with this as the
/// last error.
const ERROR_PIPE_CONNECTED: i32 = 535;

/// Default bytes reserved for the pipe's internal output buffer.
///
/// Generous relative to one audio tick's worth of data — a 20 ms tick at 48 kHz stereo `s16` is
/// under 4 KB — so a brief stall in FFmpeg reading its input doesn't immediately back-pressure
/// the writer thread.
///
/// **Not generous for video.** One 278x278 BGRA bubble overlay frame is ~309 KB, five times this
/// buffer, so every single write blocks part-way through until FFmpeg drains it — which during
/// `h264_mf`'s warm-up backlog means blocking for seconds. A caller writing frame-sized payloads
/// should size the buffer with [`NamedPipeServer::create_with_buffer`] instead.
const OUT_BUFFER_SIZE: u32 = 64 * 1024;

/// Builds the full `\\.\pipe\<name>` path FFmpeg's `-i` argument and [`NamedPipeServer::create`]
/// both need to agree on.
#[must_use]
pub fn pipe_path(name: &str) -> String {
    format!(r"\\.\pipe\{name}")
}

/// A single-client, byte-mode, write-only named pipe server.
///
/// Exists to hand FFmpeg's second `-i` input mixed PCM without writing it to disk first — the
/// same "no intermediate file" property the loopback HTTP publish server (ADR-002) chose for a
/// different reason. One instance is exactly one recording session's worth of audio; `create`
/// fails outright on a name collision rather than silently reusing a stale pipe.
pub struct NamedPipeServer {
    handle: HANDLE,
}

// `HANDLE` has no thread affinity of its own — unlike, say, a GDI device context, a pipe handle
// is safe to hand to whichever thread ends up writing to it. `windows-rs` does not derive `Send`
// automatically because *some* Win32 handle types do carry affinity; this one does not.
unsafe impl Send for NamedPipeServer {}
// `wait_for_client` and `write_all` both take `&self` and call read-only Win32 APIs against a
// handle value that never changes after construction — safe to call from different threads as
// long as the caller doesn't run them concurrently against each other, which is the caller's
// ordering responsibility (the same responsibility any `&self`-based blocking I/O API has).
unsafe impl Sync for NamedPipeServer {}

impl NamedPipeServer {
    /// Creates the pipe server. Returns immediately without waiting for a client — call
    /// [`Self::wait_for_client`] next.
    ///
    /// # Errors
    /// [`PipeError::Create`] if the pipe cannot be created — most commonly a name collision with
    /// a still-live pipe from a previous session.
    pub fn create(name: &str) -> Result<Self, PipeError> {
        Self::create_with_buffer(name, OUT_BUFFER_SIZE)
    }

    /// [`Self::create`] with an explicit output-buffer size.
    ///
    /// Size it to hold at least one whole payload. A buffer smaller than a single write means
    /// every write blocks mid-payload waiting for the reader, which turns an ordinary encoder
    /// stall into a writer thread stuck for seconds — see [`OUT_BUFFER_SIZE`].
    ///
    /// # Errors
    /// [`PipeError::Create`] if the pipe cannot be created.
    pub fn create_with_buffer(name: &str, out_buffer_bytes: u32) -> Result<Self, PipeError> {
        let path = HSTRING::from(pipe_path(name));
        // SAFETY: `path` is a valid, null-terminated wide string kept alive for the duration of
        // the call by `HSTRING`'s own ownership. The remaining arguments are plain values (byte
        // counts, a instance-count limit, `None` for default security attributes) with no
        // aliasing or lifetime requirements of their own.
        let handle = unsafe {
            CreateNamedPipeW(
                &path,
                PIPE_ACCESS_OUTBOUND,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT,
                1,
                out_buffer_bytes,
                0,
                0,
                None,
            )
        };
        if handle.is_invalid() || handle == INVALID_HANDLE_VALUE {
            return Err(PipeError::Create {
                name: name.to_string(),
                source: std::io::Error::last_os_error(),
            });
        }
        Ok(Self { handle })
    }

    /// Blocks until a client connects — FFmpeg opening `\\.\pipe\<name>` for reading.
    ///
    /// `ConnectNamedPipe` has no timeout of its own, so pairing this call with a deadline is the
    /// caller's job; `vrecnu-engine::audio_pipeline` races it against the pipeline's existing
    /// first-video-frame wait rather than blocking forever on a client that never shows up.
    ///
    /// # Errors
    /// [`PipeError::Connect`] if the wait itself fails.
    pub fn wait_for_client(&self) -> Result<(), PipeError> {
        // SAFETY: `self.handle` is a valid pipe-server handle created by `create` and not yet
        // closed — `Drop` is the only place that closes it, and it takes `&mut self`.
        let result = unsafe { ConnectNamedPipe(self.handle, None) };
        if result.is_err() {
            let err = std::io::Error::last_os_error();
            if err.raw_os_error() == Some(ERROR_PIPE_CONNECTED) {
                return Ok(());
            }
            return Err(PipeError::Connect(err));
        }
        Ok(())
    }

    /// Writes `data` to the connected client, retrying until every byte is accepted.
    ///
    /// # Errors
    /// [`PipeError::Write`] if the client has gone away (FFmpeg exited) or the write otherwise
    /// fails.
    pub fn write_all(&self, data: &[u8]) -> Result<(), PipeError> {
        let mut offset = 0usize;
        while offset < data.len() {
            let mut written = 0u32;
            // SAFETY: `self.handle` is valid per the same invariant as `wait_for_client`. The
            // slice passed is a valid, in-bounds view into `data`, and `written` is a plain
            // `u32` the call is permitted to write through.
            let result =
                unsafe { WriteFile(self.handle, Some(&data[offset..]), Some(&mut written), None) };
            if result.is_err() || written == 0 {
                return Err(PipeError::Write(std::io::Error::last_os_error()));
            }
            offset += written as usize;
        }
        Ok(())
    }
}

impl Drop for NamedPipeServer {
    fn drop(&mut self) {
        // `DisconnectNamedPipe` alone tears the connection down immediately — "This function
        // does not wait for the client process to acknowledge" (MSDN) — so a client mid-read on
        // the last bytes we wrote could see the pipe vanish instead of getting a clean EOF.
        // `FlushFileBuffers` on a named pipe blocks until the client has actually read
        // everything already written, so calling it first turns "disconnect" into "disconnect
        // once there is nothing left to lose". A client that never reads (a crashed FFmpeg)
        // would block this indefinitely; that trade only matters on the shutdown path, not the
        // steady-state write loop, and a hung FFmpeg is already a failed recording by that
        // point.
        //
        // SAFETY: `self.handle` was created by `create` in this same struct and is closed here
        // exactly once; no other code holds or uses it after `drop` runs.
        unsafe {
            let _ = FlushFileBuffers(self.handle);
            let _ = DisconnectNamedPipe(self.handle);
            let _ = CloseHandle(self.handle);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use super::*;

    #[test]
    fn a_client_reads_exactly_what_the_server_writes() {
        // Unique per test run so repeated `cargo test` invocations (or a crashed previous run)
        // never collide on a still-open pipe name.
        let name = format!(
            "vrecnu-namedpipe-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        );
        let server = NamedPipeServer::create(&name).expect("pipe creation should succeed");
        const PAYLOAD: &[u8] = b"the audio bytes FFmpeg would have received";

        // `ConnectNamedPipe` starts blocking *before* any client exists — the same order the
        // real engine uses (pipe created, `wait_for_client` on its own thread, only then FFmpeg
        // is spawned). Doing it the other way around is a real, observed bug, not a theoretical
        // one: a client that opens and reads before the server ever calls `ConnectNamedPipe`
        // hit `ERROR_PIPE_NOT_CONNECTED` here on an earlier version of this test.
        //
        // The reader uses `read_exact` for the known payload length, not `read_to_end`: FFmpeg
        // never reads "to EOF" from a live audio pipe either — it reads a known amount per
        // packet from a stream that stays open for the whole recording. `read_to_end` needs the
        // server to close first, which needs the reader to finish first: a real deadlock this
        // test hit before switching to `read_exact`, not a hypothetical one.
        let received = std::thread::scope(|scope| {
            let accept_thread = scope.spawn(|| server.wait_for_client());

            let path = pipe_path(&name);
            let reader = std::thread::spawn(move || {
                // Standing in for FFmpeg's `-i \\.\pipe\...`.
                let mut file = std::fs::File::open(&path).expect("client open should succeed");
                let mut buf = vec![0u8; PAYLOAD.len()];
                file.read_exact(&mut buf)
                    .expect("read_exact should succeed");
                buf
            });

            accept_thread
                .join()
                .expect("accept thread should not panic")
                .expect("wait_for_client should succeed");

            server.write_all(PAYLOAD).expect("write should succeed");

            reader.join().expect("reader thread should not panic")
        });

        drop(server); // must not hang even with nothing left for the client to read
        assert_eq!(received, PAYLOAD);
    }

    #[test]
    fn pipe_path_uses_the_windows_named_pipe_namespace() {
        assert_eq!(
            pipe_path("vrecnu-audio-1234"),
            r"\\.\pipe\vrecnu-audio-1234"
        );
    }
}
