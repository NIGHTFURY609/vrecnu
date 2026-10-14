# M3 — WASAPI loopback + mic audio capture

**Status:** implemented — M3 done and measured against real hardware (2026-08-13); see the M3
section of `HANDOFF.md` for results and the two bugs found only by recording for real.
*Note (2026-10-09):* where the body below calls M7's `render_args` "shipped", read "built and
unit-tested, but not yet called by anything" — M7 is only partial.
**Scope:** HANDOFF.md's M3 only — system audio + microphone capture, mixed to 48 kHz stereo
`s16le`, delivered to FFmpeg over a named pipe, wired into `vrecnu-engine::record()` and
`vrecnu-cli`'s `--mic`/`--system` flags.

Everything below assumes the code already read as context: `vrecnu-audio` (`PcmSink`,
`AudioSource`, `SilenceFiller`, `AudioError`, `SAMPLE_RATE_HZ`/`CHANNELS`/`BYTES_PER_FRAME`),
`vrecnu-encode`'s `LiveEncodeParams::audio_pipe` (already wired into `live_encode_args`),
`vrecnu-engine::pipeline.rs` (the video wiring this mirrors), ADR-007 (ffmpeg built with
`--enable-protocol=file,pipe`, audio arrives over a Windows named pipe — already decided,
not open for reconsideration here).

---

## Why this shape

The video path (`vrecnu-capture` + `vrecnu-engine::pipeline.rs`) already solved the exact
class of problem M3 has: an OS capture callback that must not block, delivered on its own
thread, consumed by a writer loop that ticks on **absolute deadlines** against a single wall
clock, with backpressure via bounded channels rather than unbounded growth. M3 reuses that
shape rather than inventing a new one — the risk profile is the same (silent-but-wrong output,
not a crash), so the mitigation should be the same.

The one real difference: video has one source, audio can have two (mic + system) that must be
**mixed** into a single stream before FFmpeg ever sees it, because `live_encode_args` accepts
exactly one `audio_pipe`.

---

## Components

### `vrecnu-audio` (pure capture + domain logic — stays platform-conditional like `vrecnu-capture`)

**New dependency:** `wasapi` crate (Windows-only), gated behind `#[cfg(windows)]` the same
way `vrecnu-capture` gates `WgcBackend`. Chosen over `cpal` because loopback capture — opening
a *render* endpoint in capture mode — is a first-class, documented feature of `wasapi`, not a
workaround; chosen over raw `windows`-crate COM calls because `vrecnu-audio`'s job is capture
logic, not COM lifecycle management, and `wasapi` is a thin, audited-by-use (CamillaDSP, etc.)
wrapper with no meaningful footprint cost.

**`src/wasapi_source.rs` (new)**
- `WasapiLoopbackSource` — opens the default render device, initializes an `IAudioClient` in
  shared mode with the loopback stream flag, in `Direction::Capture`.
- `WasapiMicSource` — opens the default capture device, shared mode, no loopback flag.
- Both implement `AudioSource`: `start()` spawns a dedicated OS thread running the WASAPI
  event-driven capture loop (`IAudioCaptureClient::GetBuffer`/`ReleaseBuffer` on the device's
  natural period) and calls `sink.on_pcm(bytes, qpc)` per packet — the callback itself only
  copies and hands off, exactly the same non-blocking contract `PcmSink`'s doc comment already
  states and `FrameSink::on_frame` already honors for video.
- Device format (sample rate, channel count, float-vs-int) comes from
  `IAudioClient::GetMixFormat()` and is **not** assumed — read at open time, same reasoning
  `vrecnu-capture` uses for reading frame size from the first frame rather than the monitor.
- `qpc` is read via `QueryPerformanceCounter` at packet-arrival time, matching the field's
  existing use in the video path (`FrameRef::qpc`).

**`src/resample.rs` (new, pure, no WASAPI dependency, fully unit-testable)**
- `pub fn to_48k_stereo_s16(input: &[u8], source_format: SourceFormat) -> Vec<u8>` (name
  indicative, finalize in the plan). Handles: linear-interpolation sample-rate conversion to
  48 000 Hz; mono → stereo by duplication; > 2 channels → first two; `f32` → `s16le` with
  `[-1.0, 1.0]` clamping before the int16 cast. Conversion happens at the source, per the
  existing doc comment in `lib.rs` ("Everything downstream assumes this format, so conversion
  happens at the source") — this file is what makes that sentence true.
- No new resampling dependency (`rubato` etc.) — linear interpolation is sufficient quality
  for voice/system-audio at these rates and keeps the binary-size budget (AGENTS.md §1) intact.

**`src/mixer.rs` (new, pure, fully unit-testable without any device)**
- `Mixer::new(sources: MixerSources)` where `MixerSources` is `Mic`, `System`, or `Both` —
  encodes at construction time which of 1 or 2 `SilenceFiller`s exist, rather than making every
  call site handle an `Option<Option<..>>` shape.
- Each active source feeds a small bounded byte queue (crossbeam `bounded`, same crate already
  used in `pipeline.rs`) fed by its `PcmSink` impl.
- `tick(&mut self, elapsed_ns: u64) -> Vec<u8>`: for each active source, drains its queue up to
  the byte count `SilenceFiller::shortfall_bytes`/`expected_samples` says should exist by now,
  fills any remaining shortfall with zeros, calls `account_for` — this is exactly
  `SilenceFiller`'s existing, already-tested contract, reused unchanged, once per source.
  When both sources are active, sums the two buffers sample-by-sample as `i16` with
  `saturating_add` (never wrapping-overflow distortion on loud combined signal) and returns the
  combined interleaved buffer; single-source configurations return that source's buffer
  directly, no summing.

### `vrecnu-engine` (wiring — owns the threads, talks to `vrecnu-encode`)

**`src/audio_pipeline.rs` (new, sibling to `pipeline.rs`)**
- Named-pipe server: `CreateNamedPipeW` with a per-session unique name
  (`\\.\pipe\vrecnu-audio-<pid>-<counter>` — avoids collision if a previous session's pipe
  hasn't been torn down yet, or a second instance runs), `PIPE_ACCESS_OUTBOUND`, byte mode.
  This is new glue code — nothing in the repo does named-pipe IPC today; `grep`-confirmed
  before writing this spec.
- Startup order (this is the audio equivalent of M2's "three things the wiring got wrong
  first" and gets the same treatment — a comment at the call site plus a test):
  1. Create the named pipe **before** spawning FFmpeg (FFmpeg's `-i \\.\pipe\...` will block on
     open until a server exists).
  2. Spawn FFmpeg (video pipe + audio pipe args both present).
  3. `ConnectNamedPipe` (blocks until FFmpeg's client-side open completes) — done on its own
     thread, concurrently with the existing `FIRST_FRAME_TIMEOUT` wait for the first video
     frame, so neither stalls the other.
  4. **`started: Instant` remains anchored to the first video frame, exactly as today.** The
     audio writer thread is spawned only once `started` is known, and its `tick()` calls use
     `started.elapsed()` — i.e., video and audio share the *one* wall clock the pacer already
     establishes, rather than each measuring its own "first sample" time. This is the specific
     drift bug this design exists to prevent; called out explicitly here so it isn't rediscovered
     the expensive way.
- Writer thread: same `sleep_until` absolute-deadline helper `pipeline.rs` already has (reused,
  not reimplemented), ticking roughly every 20 ms, calling `mixer.tick(elapsed_ns)` and writing
  the result to the pipe handle.
- `RecordRequest` gains `mic: bool` and `system: bool`. `record()` constructs
  `Some(WasapiMicSource)`/`Some(WasapiLoopbackSource)` per flag, builds the pipe path, and
  passes `audio_pipe: Some(&path)` into `LiveEncodeParams` only when at least one is set —
  `None` (today's only behavior) is unchanged when neither flag is given.

### `vrecnu-cli`
- `record()`: delete the M3 refusal block (`commands.rs` around line 266-272); pass
  `options.mic`/`options.system` through to `RecordRequest`.
- `doctor()`: replace `"audio backend     not yet implemented (M3)"` with a real probe (device
  enumeration succeeds / fails, same pattern as the existing `capture backend` and
  `h264 encoder` lines).
- Existing test `recording_with_audio_refuses_rather_than_writing_a_silent_file` (asserts the
  error contains `"M3"`) gets replaced with a real (or hardware-conditional) test once the flag
  actually works — finalize the exact replacement in the implementation plan.

---

## Error handling / explicit scope cuts

- Device disconnects mid-recording → `AudioError::DeviceDisconnected` propagates to a fatal,
  clearly-reported recording failure (mirrors how `EngineError::Capture(SizeChanged)` already
  ends a recording rather than silently freezing it). **No hot-swap/reconnect.**
- Shared-mode WASAPI only. No exclusive mode.
- No AGC, noise suppression, or echo cancellation. Raw capture + mix only —
  `loudnorm`/`eq`/trim already exist in `vrecnu-encode::args::render_args` (M7, shipped) and
  are the correct place for audio *adjustment*; M3 is capture only.
- Mixing is hard-capped at exactly two sources (mic, system). No N-source generalization —
  that's the entirety of the spec's audio scope (ARCHITECTURE.md §5).
- Unsupported device configs beyond "arbitrary sample rate, ≤ 2 meaningful channels, int or
  float samples" are out of scope; `AudioError::UnsupportedConfig` exists for whatever doesn't
  fit and the implementation should use it rather than silently degrading.

---

## Testing

**Pure unit tests (no hardware, run in CI same as today's 154):**
- `resample.rs`: rate conversion sample-count correctness, mono→stereo duplication, >2ch
  truncation, `f32→s16` clamping at and beyond ±1.0.
- `mixer.rs`: silence-fill correctness re-proven through the mixer (not just `SilenceFiller`
  standalone), saturating-add mixing on synthetic loud+loud input (confirms no wraparound),
  mic-only / system-only / both configurations each produce the expected byte count for a given
  `elapsed_ns`.
- Named-pipe server: a self-contained test where a plain thread opens and reads the pipe the
  way FFmpeg's client-side open would, confirming the create→connect→write sequence — no real
  audio device involved.

**Real hardware verification (this session, per your answer to include `--mic`):**
Structured like M2's measured done-criteria — concrete pass/fail, not "it compiled":
- `vrecnu-cli record --seconds N --system`, `--mic`, and `--mic --system` together.
- `ffprobe` on each output confirms an audio stream exists with the expected duration and
  48 kHz/stereo, matching the video track's duration (the drift this whole design exists to
  prevent).
- `ffmpeg -i out.mp4 -af volumedetect -f null -` confirms `mean_volume` is not pinned at the
  digital-silence floor (~-91 dB) for the mic take specifically, since that's the one case a
  silently-broken capture path would otherwise still produce a "plausible" file.

---

## Out of scope for this spec (tracked, not forgotten)

- M4 (camera), M5 (publish server loop), M6 (Tauri shell), M7 is already shipped
  (`render_args`), M8 (packaging), M9 (budget gates) — untouched by this change.
- RSS/footprint soak-testing for the audio path specifically — not required by
  AGENTS.md §1's existing budget language and not requested; flag if this turns out to matter
  once real numbers exist.
