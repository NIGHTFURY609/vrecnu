# vrecnu — handoff

**Last updated:** 2026-08-13
**Branch:** `feat/m3-wasapi-audio` (off `main`; M0–M2 landed on `main` first)
**Committed.** M3 (this session's work) is committed on its branch, not yet merged.

---

## Verified state, right now

Observed passing this session, not assumed:

```
apps/desktop>  cargo test --workspace -j 4     exit 0, 175 tests passed
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

### Next actions, in order

1. **Window capture has not been exercised**, only monitor capture. `window:HWND` ids
   enumerate correctly; nothing has recorded one. Expect the size to differ from the monitor
   path and the `SizeChanged` guard to matter on a window that is resized mid-recording.
2. **Multi-monitor is untested** — this machine has one display. `Monitor::from_index` is
   **1-based and rejects 0**, and `monitor_source_id` matches that rather than converting.
3. **M4: camera + bubble.** See the table below.

### Then M4–M9

| M | Scope | Notes |
|---|---|---|
| M4 | Camera + bubble integrated | `nokhwa` not yet added. Camera must be opened **exactly once** and fanned out. |
| M5 | Loopback publish server | `tiny_http` not yet added. Port negotiation, Range, token, Host/Origin checks are written and tested as pure logic; only the server loop is missing. Use `SO_EXCLUSIVEADDRUSE`, **not** `SO_REUSEADDR`. |
| M6 | Tauri shell + UI | Creates `vrecnu-shell-tauri`, the only crate allowed to name Tauri. Tray app that **destroys** its window on dismiss. |
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
- **OAuth client not registered.** Blocks any real Drive round-trip. When you do it: a **Web
  application** client, `drive.file` scope *only*, and register all five loopback origins
  `http://127.0.0.1:47821` … `:47825` at the same time (ADR-002 action item 3 — cheap now,
  annoying later).
- **The ≤150 MB idle RSS budget is at real risk with Tauri.** ADR-006 says to measure in M9
  and amend with numbers attached, and to reopen the ADR rather than reflexively raising the
  budget if the tray mitigation is not enough.
- **MP4-first (ADR-003 action item 2)** still needs verification or permanent demotion.
- **`vrecnu-shell-egui`** is a reserved crate name with a documented contract. It does not
  exist yet, and that is fine — `vrecnu-cli` is what keeps the path open.
