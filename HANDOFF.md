# vrecnu — handoff

**Last updated:** 2026-09-03
**Branch:** `main` (M0–M3 landed; M4 is this session's work, **uncommitted**)

---

## Verified state, right now

Observed passing this session, not assumed:

```
apps/desktop>  cargo test --workspace -j 4     exit 0, 212 tests passed
apps/desktop>  cargo clippy -- -D warnings      clean
apps/desktop>  cargo fmt --all -- --check       clean
apps/desktop>  scripts/check-shell-boundary.ps1 pass, incl. link-time proof
                 (pass -ManifestPath explicitly; $PSScriptRoot comes back
                  empty under some invocations and the default resolves to
                  `\..\Cargo.toml`)
```

Carried forward from 2026-08-07, not re-run this session:

```
vrecnu>        pnpm test                        5/5 packages, 135 tests passed
vrecnu>        pnpm lint / typecheck / build    all green
```

Bundle budgets (AGENTS.md §1): **82.4 KB gzip initial JS (55% of 150 KB)**, **86 KB total first
load (22% of 400 KB)**.

---

## Read these first, in this order

1. `read_this_idiot.md` — the Tauri escape hatch and how not to break it.
2. `docs/adr/006-…` — why Phase 2 uses Tauri, and the boundary that keeps it reversible.
3. `docs/adr/007-…` — the FFmpeg H.264 licensing fix.
4. `apps/desktop/README.md` — build prerequisites and the two traps that cost an hour each.
5. `TESTING.md` — what a human needs to verify by hand.

---

## Done

### Phase 1 (the PWA) — complete

Was committed as "basic functionality"; the audit found it materially unfinished.

| Area | What changed |
|---|---|
| **Blocking bug** | Permission denial hard-locked the UI forever. `PERMISSIONS_DENIED` was defined but never dispatched by any caller, and `ERROR` was rejected from `requesting-permissions`, so the reducer threw and the throw was swallowed. `ERROR` is now legal from every non-terminal state. |
| **AGENTS §10 violation** | `fix-webm-duration` read the entire recording into RAM at finalize (transiently ~2x), undoing the flat-memory guarantee in one line. Replaced with an in-place EBML header rewrite while streaming, leaving an 8-byte slot that finalize overwrites. Memory flat, finalize O(1). Dependency removed. |
| **PWA layer** | Was entirely absent. Added manifest, service worker (`vite-plugin-pwa`), generated icons (`scripts/make-icons.mjs`, no image dependency), install + update prompts, `theme-color`, configurable `base`. |
| **Trim** | Was absent. Lossless WebM Cluster-boundary cut: keep header, keep in-range Clusters, rebase timecodes, rewrite Duration. Built from `Blob.slice()` refs so a 4 GB file trims without materialising a frame. |
| **Download** | `showSaveFilePicker` with an anchor fallback. Mandated by ADR-003's amendment, previously missing — which mattered because demo mode handed users a fake `drive.google.com` link to a nonexistent file. |
| **OPFS lifecycle** | Garbage collection on reset, crash recovery for orphaned sessions, `storage.persist()`, `estimate()` headroom warning, IndexedDB fallback tier. |
| **Upload** | Chunked but could not resume: it assumed `offset = end` after a 308. Now probes the server for the real offset, honours `Range`, retries with jittered backoff, persists the session URI, exposes retry/cancel. |
| **publish/** | Was a component library that could not emit a `dist/`. Now a buildable standalone page — the artifact ADR-002 requires the exe to bundle. Includes bring-your-own-credentials. |

### Decisions recorded

- **ADR-006** — Tauri v2 over a shell-agnostic core, superseding ADR-001. Records the RSS risk
  honestly and the tray-app mitigation.
- **ADR-007** — `libx264` requires `--enable-gpl` but ADR-004 prefers LGPL; those are
  incompatible and nobody had noticed because the `configure` line was never written. Resolved
  with `h264_mf`. The full `configure` line is in the ADR.
- **MP4 demoted below WebM** in the container ladder. ADR-003 §3.3 forbids MP4-first without
  verifying Drive preview seek, which needs an OAuth client we do not have.
- **zustand removed** — AGENTS §3 lists it but it was imported nowhere. Flagged, not hidden.

### M0 — Rust workspace, complete and verified

10 crates, 128 tests. The boundary is enforced by `scripts/check-shell-boundary.ps1`, which
was proven to actually detect leaks (it fires on `serde`/`thiserror`, which are present).

### M1 — bubble window, code complete

`LayeredBubble`: own message thread, `UpdateLayeredWindow`, premultiplied-BGRA renderer,
`WM_NCHITTEST` drag. Runnable spike at `crates/vrecnu-bubble-win32/examples/bubble_spike.rs`.
**Measurements need your hardware** — see `TESTING.md`.

---

## M2 (capture → encode) — video path complete and measured

The pipeline runs end to end. Every M2 done-criterion was **measured on real hardware**
(ffmpeg 7.1.1, single 1920x1080 display), not inferred:

| Criterion | Result |
|---|---|
| 30 s 1080p30 MP4 that plays | `ffprobe`: h264 High, 1920x1080, `avg_frame_rate=30/1`, `duration=30.000000`, `nb_read_frames=900` |
| …and seeks | `-ss 22` decoded a frame from the fragmented file, exit 0 |
| `frames_dropped == 0` | 0, on every run including the soak |
| **Static desktop 10 s → exactly 300 frames** | 300 written, 231 duplicated, 0 dropped |
| **RSS flat over 5 minutes** | 72.5 MB → 72.6 MB for 65 s, one 8 MB step, then **80.5 MB dead flat for the remaining 234 s** |
| 5-minute soak output | 9000 frames written, 0 dropped, `elapsed_ms=300001`; file is `duration=300.000000`, `nb_read_frames=9000`, 46.6 MB, seeks at 4 min |

The single 8 MB step in RSS is **one 1080p frame buffer** (8.29 MB) — the sixth pool buffer
having its pages faulted in the first time demand peaked. It is bounded by the pool, which is
allocated once and never grows, and nothing moved for the following four minutes. That is the
property under test: not "never allocates", but "does not climb".

The image was checked visually, not only numerically: a decoded frame is correctly coloured
and unskewed, which is what proves the row-pitch copy and the BGRA channel order are both
right. A wrong stride produces a *plausible* file — this is the only check that catches it.

### What was built

- **`vrecnu-encode/src/process.rs`** — `FfmpegProcess`: spawn with `CREATE_NO_WINDOW`, frame
  writes to stdin, a stderr reader thread feeding `ProgressParser`, a bounded stderr tail,
  `finish()` (close stdin → EOF → clean flush), `abort()`, and `failure_reason()`. Plus
  `probe_h264_encoder`, which parses the *name column* of `-encoders` rather than substring
  matching, so `h264_mf`'s description mentioning "h264" cannot invent an encoder.
- **`vrecnu-engine/src/pipeline.rs`** — the wiring: capture callback → pooled buffer →
  bounded channel → pacer tick → `ffmpeg` stdin. Also `list_capture_sources`, `probe_encoder`.
- **`vrecnu-cli`** — `sources` and `record` are real; `doctor` now probes capture and the
  encoder instead of printing "not yet implemented". `shell.rs` implements `ShellServices`
  for real, which strengthens the ADR-006 evidence rather than just satisfying a signature.
- **`vrecnu-capture`** — `primary_monitor_source_id()`, and `NoFrames` / `SizeChanged` errors.

### Three things the wiring got wrong first, all of which produce a plausible file

Each of these is now a comment at the site and a test:

1. **The clock starts at the first frame, not at `start()`.** WGC takes tens of ms to deliver
   anything; starting the pacer earlier spends ticks with nothing to send and a "10 second"
   recording comes out 299 frames long.
2. **The frame size comes from the first frame, not from the monitor.** `-video_size` is fixed
   at spawn, and a display that reports 1920x1080 while the surface is 1920x1088 shifts every
   row. The pool is therefore allocated inside the sink, on the first frame.
3. **Deadlines are absolute, never `now + interval`.** Folding oversleep back in gives a
   correct duration with motion running fast.

### Two real FFmpeg bugs found by running it

Both failed at *encoder-open / header-write* time — i.e. after the user believes recording has
started — and both were invisible to the existing unit tests, which only asserted on strings:

- **`-profile:v high` is rejected by `h264_mf`.** It registers `profile` as a bare int with no
  named constants. Now `H264Encoder::profile_value()`: `100` for MF, `high` for openh264.
- **`default_base_is_moof` is not a movflag.** The flag is spelled **`default_base_moof`**;
  FFmpeg's *description* is "set the default-base-is-moof flag", which is where the wrong
  spelling came from. The muxer rejected the whole `-movflags` line and nothing was written.

### Known, measured, not yet addressed

- **The encoder is slow to warm, and the loop recovers from it.** `h264_mf` starts around
  14 fps and takes about a minute to reach 30.0, so short recordings finish slightly late:
  30 s of frames took 30.17 s, 10 s took 10.23 s. This is *not* cumulative drift — over
  5 minutes it came back to `300.001 s` for 9000 frames, because deadlines are absolute and
  the loop catches up once FFmpeg keeps pace. A short clip's motion therefore plays ~0.5 %
  fast; a long one is exact. Worth revisiting in M9 with `-scenario display_remoting` (an MF
  tuning mode meant for exactly this workload — verified to run, but one machine is not
  evidence, so it was left out).
- **Audio makes the warm-up backlog worse, not just present.** The extra WASAPI/writer threads
  compete with `h264_mf` for CPU during exactly the window it's already slow, so a short
  `--mic`/`--system` recording's `ffmpeg.finish()` can take several **seconds** longer than the
  same recording without audio — still correct on completion (see M3), just slower to get
  there. Same root cause as the row above, not a new one.

---

## M3 (WASAPI loopback + mic) — audio path complete and measured

`--mic`, `--system`, and both together all produce a valid MP4 — **measured against real
hardware** (real microphone, a real 440 Hz tone played through the default output device for
loopback), not inferred from unit tests alone:

| Criterion | Result |
|---|---|
| `--system` produces h264 + AAC, 48 kHz stereo | `ffprobe`: both streams present, `sample_rate=48000`, `channels=2` |
| …and the tone is actually audible, not silence | `volumedetect`: `mean_volume=-32.9dB`, `max_volume=-20.2dB` (digital silence floors at ~-91dB) |
| `--mic` produces the same, with real ambient/room audio | `mean_volume=-44.3dB`, `max_volume=-27.3dB` |
| `--mic --system` mixes both into one stream | `mean_volume=-43.0dB`, `max_volume=-17.3dB` — louder peak than either alone, consistent with the two sources summing |
| All three configurations complete and produce a playable file | confirmed on every run this session, after the two fixes below |

### What was built

- **`vrecnu-audio`** — `resample.rs` (linear resample to 48 kHz stereo `s16le`, mono→stereo
  duplication, >2ch truncation, `f32`/`s16` conversion — pure, unit-tested, no hardware
  needed), `mixer.rs` (combines up to two `SilenceFiller`-backed sources; both fillers always
  tick on the same `elapsed_ns`, which guarantees equal-length output so summing two sources
  never has to guess about a length mismatch), `wasapi_source.rs` (`WasapiMicSource` /
  `WasapiLoopbackSource` on the `wasapi` crate, one dedicated thread per source).
- **`vrecnu-namedpipe`** (new crate) — a minimal Win32 named-pipe server. Isolates the
  unavoidable `unsafe` FFI the same way `vrecnu-bubble-win32` does: confined to one file,
  `unsafe_op_in_unsafe_fn` denied, safe public API. Registered in
  `check-shell-boundary.ps1`'s crate list.
- **`vrecnu-engine/src/audio_pipeline.rs`** — `AudioPipeline`: starts WASAPI source(s), the
  mixer, and the named pipe as one RAII unit. `Drop` stops everything, so `run_until_stopped`'s
  several `?`-driven exit paths need no manual cleanup — just a variable declared before
  anything fallible. `RecordRequest` gained `mic`/`system` bools.
- **`vrecnu-cli`** — `--mic`/`--system` work for real now (the M3 refusal is gone); `doctor`
  reports real device availability via a cheap WASAPI enumeration probe (confirmed live: `audio
  backend     mic ready, system ready`).

### Two real bugs found only by recording for real

Both were invisible to every unit test — the mixer/pipe round-trip tests were green the whole
time. Found by actually running `--system`/`--mic` against real hardware, the M2 pattern
repeating exactly:

1. **WASAPI loopback direction was backwards.** `initialize_client` needs a *mismatched*
   direction from the device to get `AUDCLNT_STREAMFLAGS_LOOPBACK` set — render device, capture
   client — confirmed against the `wasapi` crate's own source. Passing `Render` for both, which
   reads more naturally, is what `--system` shipped with first and it failed on real hardware
   with `AUDCLNT_E_UNSUPPORTED_FORMAT` every time.
2. **Shutdown order deadlocked FFmpeg.** `ffmpeg.finish()` closes the video stdin and waits for
   the process to exit, but the audio named pipe was left open and actively written to until
   `AudioPipeline` dropped — which happens *after* `run_until_stopped` returns, which is after
   `finish()` was supposed to already have returned. A muxer reading two live inputs has no
   reason to finalize while one of them is still open. Fixed by stopping audio (flush,
   disconnect, close — the audio-side equivalent of closing stdin) *before* calling `finish()`,
   not after. Without this, `finish()` never returned — not a deadlock inside FFmpeg or this
   process, just two inputs each waiting on something only the other one could have provided.

### Known, measured, not yet addressed

- **Audio/video CPU contention compounds `h264_mf`'s warm-up slowness** (already noted under
  M2's own known-issues, since it's the same underlying cause). `AudioPipeline::stop`'s
  `STOP_GRACE` bound (8s) is sometimes not enough during a slow warm-up — confirmed harmless
  when it fires (the write finishes moments later regardless, and the recorded file is still
  correct — verified with `ffprobe`/`volumedetect`), so the bound stays firm rather than
  growing indefinitely to chase a delay that varies by machine.
- **`ConnectNamedPipe`/a blocked `WriteFile` have no portable cancel-from-another-thread short
  of overlapped I/O**, which `vrecnu-namedpipe` does not use. The one pathological case this
  matters for — FFmpeg dying between spawn and opening its audio input — is bounded by
  `STOP_GRACE` rather than fixed properly; see that constant's doc comment for the trade-off.

---

## M4 (camera + bubble) — complete and measured

`--camera` burns a circular camera bubble into the recording; `--bubble` also floats it over the
desktop. Verified on real hardware (a USB webcam, a real screen), not inferred from unit tests:

| Criterion | Result |
|---|---|
| `--camera 0` produces a playable 1080p30 MP4 | `ffprobe`: h264, 1920x1080, `avg_frame_rate=30/1`, `duration=10.07`, 302 frames |
| The camera is actually *in* it, correctly cropped and coloured | frame extracted and looked at: circular crop, white ring, soft shadow, natural skin tones |
| …and is still there at the end of the file | last 0.5 s extracted: bubble present, so the shutdown reorder does not truncate the overlay |
| `frames_dropped == 0` (AGENTS §1) | 0, on three consecutive 10 s runs |
| `--bubble` floats a window *and* records it exactly once | one bubble in frame, at the window's position |
| `--camera --bubble --mic --system` all at once | h264 + aac 48 kHz stereo, valid file, bubble present |
| M3 audio not regressed by the shutdown reorder | `--mic --system`: `mean_volume=-40.6 dB`, `max_volume=-7.5 dB` — real audio, not silence |

### What was built

- **`vrecnu-camera`** — `format.rs` (`select_format`, pure and hardware-free: smallest format that
  still covers the bubble, ties broken by frame rate, largest as a fallback) and
  `nokhwa_source.rs` (`NokhwaCameraSource` on Media Foundation, one device open fanned out to N
  consumers, RGBA→BGRA in place, error classification into denied/busy/backend).
- **`vrecnu-bubble-win32`** — `BubbleMask` precomputes the supersampled circle once instead of per
  frame, and `AlphaMode` makes the premultiplied/straight distinction an explicit argument.
  `premultiply_into` derives the window's surface from the pipe's, so a frame is composited once
  rather than twice. `present` takes `&[u8]` and double-buffers, so presenting allocates nothing.
- **`vrecnu-engine/src/camera_pipeline.rs`** — `CameraPipeline`: device, bubble window and named
  pipe as one RAII unit, mirroring `audio_pipeline`. Ticks the overlay pipe on the recording's
  shared clock.
- **`vrecnu-encode`** — `LiveEncodeParams::camera_position`, so the burn-in lands where the user
  sees the bubble rather than always in a corner; `finish()` split into `close_stdin()` + `wait()`.
- **`vrecnu-cli`** — `cameras`, `--camera [ID]`, `--bubble`; `doctor` reports real camera counts.

### Five bugs found only by running it

The M2/M3 pattern held: every one of these passed the unit tests.

1. **All recording was broken on this machine, before M4 touched anything.**
   `windows-capture` 2.0's `Monitor::index()` parses the number out of the device name
   (`\\.\DISPLAY5` → 5) while `Monitor::from_index(n)` returns `enumerate()[n - 1]`, a *position*.
   Both are one-based, which is what let the confusion survive review — but they are different
   namespaces. This laptop's single panel is `\\.\DISPLAY5`, so `sources` listed `monitor:5` and
   starting it reported the display had been disconnected. Resolution now matches on the device
   number (`is_monitor_source`/`monitor_by_index`), which is also order-independent and therefore
   what multi-monitor needs.
2. **The bubble was drawn twice whenever the window was shown.** DWM composites a layered topmost
   window onto the desktop, so WGC records the bubble on its own; burning it in as well produced
   two visibly offset circles — the captured copy sits where the window is, the overlay sits at
   the coordinates FFmpeg got at spawn. `CameraSelection::burn_in` now skips the overlay entirely
   when a bubble window is over a captured *monitor*, and keeps it for window capture (where the
   bubble is a separate window and genuinely absent) and for headless `--camera`.
3. **`frames_dropped` counted frames nobody wanted.** The counters ran from before FFmpeg was
   spawned until after it had exited. WGC delivers throughout both windows and nothing drains the
   channel, so the pool exhausts and every frame looks lost: 67 phantom drops at the tail, 3 at the
   head. `PipelineSink::counting` limits the statistics to the pacer's first tick through its last.
4. **Camera shutdown deadlocked against FFmpeg, the M3 audio bug through a different door.** The
   overlay writer blocks in `WriteFile` because FFmpeg, still behind on `h264_mf` warm-up, has
   stopped draining the pipe — and FFmpeg only catches up once its video stdin closes, which
   happened *after* the camera was stopped. Cost 5 s of `STOP_GRACE` on every recording. Fixed by
   splitting `finish()` so stdin closes **first**, the pipes close **second**, and the process is
   waited on **last**. An attempt to force the issue with `DisconnectNamedPipe` instead made it
   worse — an indefinite hang — and was reverted.
5. **The pipe buffer was smaller than one frame.** 64 KB is generous for a 4 KB audio tick and
   five times too small for a ~309 KB bubble surface, so every camera write blocked mid-payload.
   `create_with_buffer` sizes it to two whole frames.

### Known, measured, not yet addressed

- **Audio still costs frames at startup.** `--mic --system` drops ~26 frames, all of them before
  the first progress line — FFmpeg takes seconds to become ready with an audio input and is not
  reading stdin yet. Pre-existing (this is the M2/M3 warm-up contention), only now *visible*
  because the drop counters are honest. Camera-only and screen-only both reach 0. Belongs with the
  M9 encoder-tuning work, not with M4.
- **Dragging the bubble does not move the burn-in.** `overlay=x=..:y=..` is fixed when FFmpeg
  spawns. Only reachable through window capture now, since monitor capture no longer burns in.
- **MJPEG-only cameras are refused** at negotiation rather than decoded, so `nokhwa/decoding` and
  its NASM build prerequisite stay out of every contributor's build. `--features mjpeg` enables it.
- **`nokhwa` pulls in `image`, and it costs less than it looks.** Measured rather than assumed:
  `cargo build --release -p vrecnu-cli` produces a **0.93 MB** exe — release, stripped, no FFmpeg
  linked, which is exactly the shape AGENTS §1 budgets at ≤ 15 MB. That binary links the entire
  engine, `nokhwa` and `image` included, so the camera stack is ~6 % of the budget and is not
  where the risk lives. What is still unmeasured is the **Tauri shell** (M6), which is what
  ADR-006 actually warns about; M9 owns that measurement, not this one.
  - `mozjpeg-sys` and its NASM build requirement stay out of the default build, confirmed with
    `cargo tree -i mozjpeg-sys` → "did not match any packages". The `Cargo.lock` entries for it
    are inert: reachable only via `--features mjpeg`.

## M5 (loopback publish server) — complete and exercised against a real process

The last piece of the record→link pipe that lives on our side of the browser. `tiny_http` on a
hand-built socket, the Vite build of `packages/publish` embedded at compile time, and the
engine's `begin_publish` / `abort_publish` / `submit_publish_result` wired to it.

### What was built

- **`vrecnu-publish-server`** gained `build.rs` (embeds `packages/publish/dist`), `assets.rs`
  (the lookup table — deliberately not a document root) and `server.rs` (the loop, routing, the
  two security checks, Range, the three `POST /api/*` endpoints).
- **`CapabilityToken::random`** — 32 bytes from `getrandom`, hex. Hex rather than base64url
  because the token travels in a URL *and* a JSON body, and hex needs no escaping in either.
- **`vrecnu-engine`** — a `PublishPump` thread that **owns** the `PublishServer`, turns
  `PublishSignal` into `EngineEvent`, puts the share link on the clipboard, and releases the
  port when it exits. Plus `Session::adopt_recording`, so a publish need not follow a record in
  the same process.
- **`vrecnu-cli publish <FILE>`** — a real command now, and `CliShell::open_external_url`
  actually launches the browser. This is what makes the Drive round trip testable by a human
  the moment an OAuth client exists.

### Three things that were wrong until it was actually run

1. **`shutdown()` did not release the port.** The `Arc<tiny_http::Server>` outlived it, so the
   listener stayed bound until the whole struct dropped. Stopping the accept loop is not the
   same as closing the socket.
2. **Even after fixing that, release was asynchronous.** tiny_http's accept thread unwinds on
   its own schedule, so `shutdown()` returned while the port was still bound. Invisible on an
   idle machine; reproducible under `cargo test --workspace`, where the whole box is busy.
   `shutdown` now re-binds the port to confirm the OS agrees, with a 1.5 s grace. The failure
   this prevents is the *next* publish reporting `AllPortsBusy` with nothing listening.
3. **The hand-rolled test client read a body from a `HEAD` response.** Not a server bug, but it
   is exactly the mistake the publish page could make: a `HEAD` carries the `Content-Length` of
   the body it *would* have sent — which is the whole reason `probeRangeSourceSize` uses it —
   and no body.

### Observed, 2026-09-04

- **240 tests** workspace-wide (was 224), `clippy -D warnings` exit 0, `cargo fmt --check`
  clean, shell-boundary check green across all 11 crates plus the `vrecnu-cli` link proof.
- `tests/loopback.rs` drives a real `PublishServer` over a real socket: index and hashed asset,
  `/api/session`, whole-file `GET`, `Range: bytes=100-199` → 206 with the exact bytes,
  `bytes=99999-` → 416 with `Content-Range: bytes */4096`, wrong token → 403 everywhere, a
  rebound `Host` → 421, a foreign `Origin` → 403, progress and completion POSTs, 410 after the
  session ends, and the port free after shutdown.
- **A real `vrecnu-cli publish` process**, driven from PowerShell: bound 47821, served the
  actual Vite build (`index.html` 635 B, `index-DUP3FnI7.js` 237 KB, correct `Content-Type`s,
  **no `Access-Control-Allow-Origin` on any response**), `/api/session` returned the metadata
  with no on-disk path in it, `HEAD /r/<token>` reported `Content-Length: 5000` and
  `Accept-Ranges: bytes`, a progress POST printed `uploaded 50%`, a completion POST printed the
  share link, and the process exited with 47821 free.

### Known, measured, not yet addressed

- **The completion in that run carried a fake file id.** Nothing has touched Google. This is
  still blocked on registering an OAuth client — see the open item below.
- **The page has never been rendered in a browser against this server.** The bytes are right;
  React mounting and fetching was not observed.
- **`vrecnu-cli.exe` has not been re-measured** since it started embedding 237 KB of JS. The
  ≤ 15 MB budget is in no danger, but the 0.93 MB figure is now stale.
- **`CliShell` cannot copy to the clipboard** and says so. The engine logs the warning and
  still reports the link, which is correct — a clipboard failure must not downgrade a
  successful upload. Real clipboard delivery is an M6 question.
- **Multi-range requests are declined**, not half-implemented: the server sends the whole file,
  which RFC 9110 allows.

---

### Next actions, in order

1. **Register the OAuth client and run the round trip.** Everything on our side is now built
   and proven; `vrecnu-cli publish <FILE>` is a one-command way to try it. This is the single
   highest-value action left in the project and no amount of code substitutes for it.
2. **M6: the Tauri shell.** Fully unblocked — `RecorderEngine` is implemented end to end.
3. **Window capture with a camera has not been exercised** — it is the one path that still burns
   the overlay in, and the one where `camera_position` still matters.
4. **Multi-monitor is still untested**, though the fix in bug 1 above is what it needed.

### Then M6–M9

| M | Scope | Notes |
|---|---|---|
| M5 | Loopback publish server | **Done.** See the M5 section above. |
| M6 | Tauri shell + UI | Creates `vrecnu-shell-tauri`, the only crate allowed to name Tauri. Tray app that **destroys** its window on dismiss. Needs `set_clipboard_text` and `open_external_url` for real, and a `PublishSurface` — the engine already emits `PublishOpened`/`PublishProgress`/`PublishDone`. |
| M7 | Editing | `render_args()` already builds the command lines and is tested. |
| M8 | Packaging | NSIS `installMode: "currentUser"`, `tauri-plugin-updater`, stripped FFmpeg fetched + SHA-256 verified. **Blocked on a code-signing cert** (ADR-005 action item 5). |
| M9 | Budget gates | Measure RSS and amend AGENTS §1 with real numbers. |

---

## Environment gotchas that cost real time

1. **Windows SDK is required** and its absence is confusing — `link.exe` exists without it, so
   the error looks like a project problem. `cargo check` fails too, because proc-macro crates
   compile for the host. Symptom: `LNK1181: cannot open input file 'kernel32.lib'`.
2. **Do not run cargo from Git Bash.** Its `/usr/bin/link` (GNU coreutils) shadows MSVC's
   `link.exe` and gives a nonsense `link: extra operand` error. Use PowerShell.
3. **`cargo test -j 2` if the build OOMs.** 16 parallel `rustc` jobs against low free commit
   dies with `memory allocation of N bytes failed` / `STATUS_STACK_BUFFER_OVERRUN`. Common
   right after a VS/SDK install while `TiWorker` and Defender are still indexing.
4. **`rust-toolchain.toml` says `stable`, not a pinned version** — pinning makes rustup fetch
   that exact channel and fail offline. `rust-version = "1.89"` in Cargo.toml is the real floor.
5. **PowerShell 5.1 reads UTF-8 without a BOM as ANSI.** Keep `.ps1` files ASCII-only or they
   fail to parse. Also do not set `$ErrorActionPreference = 'Stop'` around cargo: its ordinary
   progress output goes to stderr and gets turned into a terminating error.
6. **`Select-Object -First N` truncates a native command's pipeline** and corrupts
   `$LASTEXITCODE`. Capture to a variable with `Out-String` before filtering.
7. **PowerShell shows only the first line of a native command's stderr** as the error record,
   and swallows the rest. A multi-line error message loses everything after line one — which
   is exactly where FFmpeg's explanation lives. Run the exe through `cmd /c "... > file 2>&1"`
   when diagnosing, not `cargo run` through the PowerShell pipeline.
8. **A broken pipe from FFmpeg tells you nothing.** `FfmpegProcess::failure_reason` exists for
   this: on a write failure it reaps the child and prefers FFmpeg's own exit code and stderr
   tail over `BrokenPipe`. Both real bugs found this session were invisible until it did.

---

## Open items carried forward

- **ADR-005 action item 5 — code-signing certificate custody.** Unresolved and **blocks M8**.
- **OAuth client not registered.** Blocks any real Drive round-trip, and is now the *only*
  thing blocking it: as of M5 everything up to the browser calling Google works against a real
  socket. When you do it: a **Web application** client, `drive.file` scope *only*, and register
  all five loopback origins `http://127.0.0.1:47821` … `:47825` at the same time (ADR-002
  action item 3 — cheap now, annoying later). Then:
  `pnpm --filter @vrecnu/publish build` and `cargo run -p vrecnu-cli -- publish <FILE>`.
- **The ≤150 MB idle RSS budget is at real risk with Tauri.** ADR-006 says to measure in M9
  and amend with numbers attached, and to reopen the ADR rather than reflexively raising the
  budget if the tray mitigation is not enough.
- **MP4-first (ADR-003 action item 2)** still needs verification or permanent demotion.
- **`vrecnu-shell-egui`** is a reserved crate name with a documented contract. It does not
  exist yet, and that is fine — `vrecnu-cli` is what keeps the path open.
