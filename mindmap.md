# vrecnu — codebase mindmap

**A map of what exists right now**, as of 2026-10-15 (M0–M5 and M6 S1+S2 are all on `main`; M6 S1+S2 merged locally, not yet pushed; M5 merged via
PR #5). Not a plan — `ARCHITECTURE.md` is the plan and `HANDOFF.md` is the running log and the
**status of record**: this map summarises it and is updated in the same change (`AGENTS.md` §11). This
document answers one question: *what is actually in this repository, and how much of it works?*

## Status legend

| Mark | Meaning |
|---|---|
| **✅ measured** | Built, and verified against real hardware or a real run — evidence in `HANDOFF.md` |
| **🟩 tested** | Built, with unit tests passing. Logic proven; no hardware or end-to-end run behind it |
| **🟨 written** | Code exists and compiles, but nothing calls it yet |
| **⬜ absent** | Does not exist |

The distinction between 🟩 and ✅ is the one that keeps burning this project. Every milestone so
far has shipped green unit tests **and** a bug that only appeared when the thing was actually
run — a backwards WASAPI loopback direction, a misspelled FFmpeg movflag, two shutdown
deadlocks. Treat 🟩 as "not yet known to work."

---

## The one-screen map

```
vrecnu/
├── packages/            TypeScript — Phase 1 PWA + the shared publish page
│   ├── core/       🟩   recorder state machine (163 LOC)
│   ├── storage/    🟩   StorageProvider + Google Drive, resumable upload (1,092)
│   ├── publish/    🟩   the OAuth/upload page BOTH phases share (985) — served by the
│   │                    Rust loopback server (bytes checked over HTTP; never rendered
│   │                    in a browser, never run against real Drive)
│   ├── ui/         🟩   shadcn-style component seed (57)
│   ├── desktop-ui/ 🟨   the Tauri shell's webview UI (M6): engine TS mirror, snapshot-seeded
│   │                    reducer, diagnostic screen; the only package importing @tauri-apps/*
│   └── web/        🟩   the PWA: record, OPFS streaming, trim, download (3,992)
│
└── apps/desktop/        Rust — Phase 2, 12 crates, ~13,600 LOC, 252 tests
    ├── vrecnu-types         🟩  shared vocabulary, zero deps           (501)
    ├── vrecnu-shell-api     🟩  THE BOUNDARY: 4 traits, no GUI names   (469)
    ├── vrecnu-capture       ✅  Windows Graphics Capture + pacer/pool  (793)
    ├── vrecnu-audio         ✅  WASAPI loopback + mic, resample, mix  (1,147)
    ├── vrecnu-camera        ✅  webcam, opened once, fanned out        (768)
    ├── vrecnu-bubble-win32  🟩  floating layered-window bubble; M1 checks pending (1,210)
    ├── vrecnu-encode        ✅  FFmpeg child process + arg building  (1,276)
    ├── vrecnu-namedpipe     ✅  Win32 named pipes (audio/camera → ffmpeg) (273)
    ├── vrecnu-publish-server ✅ loopback server, running and driven  (1,460)
    ├── vrecnu-engine        ✅  orchestration; owns every thread     (3,400)
    └── vrecnu-cli           ✅  headless shell, the ADR-006 proof,
                                 and now a working `publish` command    (860)

    vrecnu-shell-tauri   🟨  S1 tray shell (tray, on-demand window, single instance) + S2
                                 (ShellServices, event pump, read-only IPC) (683);
                                 no recorder UI yet; tray checks passed 2026-10-15
    vrecnu-shell-egui    ⬜  reserved name, documented contract, intentionally absent
```

---

## The Rust core, layer by layer

### The boundary (`vrecnu-shell-api`)

The most important 469 lines in the repo. Four traits — `RecorderEngine`, `ShellServices`,
`BubbleSurface`, `PublishSurface` — plus one flat `AppError`. **Nothing here may name a GUI
framework.** That rule is what let [ADR-006](docs/adr/006-phase-2-tauri-shell-with-swappable-core.md)
re-admit Tauri without betting the project on it, and it is enforced twice in CI by
`scripts/check-shell-boundary.ps1`: a `cargo tree` scan of all 11 shell-free crates, **plus** a
link-time proof that `vrecnu-cli` still builds with no Tauri in its graph.

> `RecorderEngine` had **no implementation at all** until M5. The trait every shell
> sits on was a contract nothing fulfilled — the engine offered only blocking free functions,
> which suit `vrecnu-cli` and are unusable from a UI that must keep repainting.
> `vrecnu-engine/src/session.rs` now implements it, publish path included. Committed as
> `01108e2`, on `main` via PR #5.

### Capture → encode (the ✅ spine)

```
WGC callback ─► pooled buffer ─► bounded channel ─► pacer tick ─► ffmpeg stdin ─► .mp4
   (~2ms)         6 frames,        backpressure      absolute        child proc
                 never grows        = drop signal     deadlines
```

Measured end to end: 5-minute soak, 9000 frames, **0 dropped**, RSS flat at 80.5 MB for the
last four minutes, output seekable at 4 min. Three wiring bugs were found here that each
produce a *plausible* file rather than an error — the clock must start at the first frame, the
frame size must come from the pixels not the monitor, and deadlines must be absolute. All three
are now a comment and a test at the site.

### Audio (`vrecnu-audio` + `vrecnu-namedpipe`)

Mic and/or WASAPI loopback → resample to 48 kHz stereo `s16le` → mix → named pipe → FFmpeg's
second input. Verified with `volumedetect` against a real 440 Hz tone, not just "a file
appeared": `-32.9 dB` mean for system audio against a ~-91 dB digital-silence floor.

### Camera + bubble (`vrecnu-camera` + `vrecnu-bubble-win32`)

The device is opened **exactly once** and fanned out to two consumers — the floating bubble and
the burned-in overlay. This is not tidiness: Windows usually multiplexes a second open, but
virtual and OEM cameras (OBS Virtual Cam, vendor "beauty filter" drivers) take exclusive control
and fail it — *after* the user has pressed record.

The bubble is a raw Win32 layered window on its own thread, not a second `winit` window (a
second `tao`/`winit` loop in one process is unsupported) and not a webview. `UpdateLayeredWindow`
from a premultiplied-BGRA DIB gives true per-pixel alpha.

### Publish (`vrecnu-publish-server`) — the ADR-002 loopback origin

```
begin_publish ─► bind 47821..47825 ─► serve index.html + /r/<token> ─► POST /api/complete
                 SO_EXCLUSIVEADDRUSE   same origin, zero CORS headers    ─► clipboard ─► port freed
```

The Rust side serves one file and waits to be told a link exists. **It never sees an OAuth
token, a Drive request, or a byte of the upload** — that all happens in `packages/publish`,
running in a browser, exactly as in Phase 1.

Four things here are load-bearing and none of them are obvious:

- **`SO_EXCLUSIVEADDRUSE`, not `SO_REUSEADDR`.** Without it another local process can bind the
  same `127.0.0.1:47821` and serve its own page on an origin the user's Google account has
  authorised. `SO_REUSEADDR` would *create* that hole. Neither std nor socket2 exposes the
  option, so this is the crate's one `unsafe` — which is why its lint is `deny`, not `forbid`.
- **The `Host` check is the DNS-rebinding defence.** Binding loopback does not provide it: a
  hostile page can point its own name at `127.0.0.1` and then treat our replies as same-origin
  to itself.
- **Shutdown waits for the OS to agree the port is free**, by re-binding it. Dropping the
  listener is not the same thing — tiny_http's accept thread unwinds asynchronously — and the
  observable consequence was the *next* publish reporting `AllPortsBusy` on a machine where
  nothing was listening. Found by running two tests at once, not by reading the code.
- **The page is embedded, never a document root.** `build.rs` compiles `packages/publish/dist`
  into a lookup table, so there is no directory to traverse out of. A missing `dist/` is not a
  build failure — a Rust contributor should not need Node — it becomes `AssetsMissing` at
  runtime.

### Orchestration (`vrecnu-engine`)

The largest crate, and the only one that knows how the others fit together. Owns the pipeline,
the audio and camera sub-pipelines, the session state machine, the single error→`AppError`
conversion, and now the `RecorderEngine` implementation.

---

## The TypeScript side

| Package | State |
|---|---|
| `web` | The PWA. Records, streams chunks to OPFS with flat memory, trims losslessly at WebM Cluster boundaries, downloads via `showSaveFilePicker`. Service worker, install/update prompts, generated icons. |
| `storage` | `StorageProvider` interface + Google Drive. Resumable upload that **probes the server for the real offset** after a 308 rather than assuming `offset = end`, with jittered backoff and a persisted session URI. |
| `publish` | The one OAuth implementation, shared by both phases — the PWA uses it directly, the desktop will load it in a web view. GIS token client, no client secret, bring-your-own-credentials. |
| `core`, `ui` | State machine and component seed. Small and stable. |

**All of it is 🟩, not ✅.** Drive is exercised only against a fake. See the gap below.

---

## Milestone ledger

| | Scope | State |
|---|---|---|
| **Phase 1** | The PWA | Code complete — **never run against real Drive** |
| **M0** | Workspace + shell boundary | ✅ |
| **M1** | Bubble window | 🟩 code; the `TESTING.md` §1 measurements need your hardware (the bubble has since floated and recorded correctly in M4's run, but the always-on-top / focus / CPU / DPI checks are unrecorded) |
| **M2** | Capture → encode | ✅ measured |
| **M3** | WASAPI audio | ✅ measured |
| **M4** | Camera + bubble | ✅ measured |
| **M5** | Loopback publish server | ✅ **measured** — real process, real socket: page served, `Range` honoured, progress and share link round-tripped, port released. Drive itself still unexercised (the completion POSTs were fake), and the page has never been rendered in a browser against the server. On `main` (PR #5) |
| **M6** | Shell + UI | 🟨 **in progress.** Design approved 2026-10-14 (`docs/superpowers/specs/2026-10-09-m6-tauri-shell-design.md`); **S1+S2 done** (tray shell, on-demand window, single instance; TauriShell, read-only IPC, diagnostic screen — 2026-10-15; tray checks passed 2026-10-15); S3 planned 2026-10-15 (`docs/superpowers/plans/2026-10-15-m6-s3-recorder.md`); S4–S5 next. Engine side and toolchain both unblocked (2026-10-14) |
| **M7** | Editing | 🟨 `render_args()` builds and tests the command lines; nothing runs them |
| **M8** | Packaging, installer, updater | ⬜ **blocked** on a code-signing certificate |
| **M9** | Budget gates | ⬜ |
| **Phase 3** | Timeline editor, multi-provider storage, effects, viewer page | ⬜ |

---

## What is verified, and what only looks like it

**Observed passing, 2026-10-15** (clippy, tests, fmt and the boundary script re-run after the Tauri shell landed):
**252 Rust tests**, `clippy --all-targets --all-features -D warnings`, `cargo fmt --check`, and
the shell-boundary dependency check across the 11 shell-free crates plus the `vrecnu-cli` link-time proof. Release `vrecnu-cli.exe` was **0.93 MB** before M5 — the exact
shape `AGENTS.md` §1 budgets at ≤ 15 MB — and has **not been re-measured** since the publish
page (237 KB of JS) started being embedded in it.

**M5 observed against a real running process**, not only in tests: `vrecnu-cli publish <file>`
bound 47821, served the real Vite build (`index.html` 635 B, `index-*.js` 237 KB, correct
`Content-Type`s, **no `Access-Control-Allow-Origin`**), answered `/api/session` with the right
metadata and no on-disk path, reported `Content-Length: 5000` to a `HEAD /r/<token>`, took a
progress POST and a completion POST, printed the share link, and released the port.

**TypeScript, observed passing 2026-10-15:** `pnpm typecheck`, `pnpm lint` and `pnpm test`
(177 tests across 6 packages — a count that moves as tests are added or removed); the web and
publish Vite builds succeed; the web bundle budget holds at about 82 KB gzip initial JS (of
150 KB).

**Not verified by anything:**

- The **Drive round-trip**. No OAuth client is registered, so upload, share-permission and
  `webViewLink` have never touched Google. `docs/oauth-setup.md` is the runbook; it needs a
  human with the account. M5 moved the boundary of this gap but did not close it: everything
  *up to* the browser calling Google now works against a real socket, and the completion POST
  in that run carried a **fake** file id.
- **The publish page has never been rendered in a browser against the loopback server.** The
  bytes were verified with an HTTP client; React actually mounting and fetching was not.
- **Idle RSS with a WebView.** ADR-006's live risk: ~80–150 MB against a ≤ 150 MB budget. The
  mitigation (a tray app that *destroys* its window on dismiss) is a plan, not a measurement.
- **Multi-monitor**, and **window** capture with a camera. One display here; `Monitor::from_index`
  is 1-based and rejects 0.

---

## The gaps that actually matter

1. **The product's magic moment has never happened.** "Record → link on your clipboard" is the
   entire pitch, and the Drive half has only ever run against a fake. The pipe to Drive is now
   built and proven end to end on this side of the browser; the browser half **needs your
   Google account** — nothing here can do it. `vrecnu-cli publish <file>` is the way to try it
   the moment an OAuth client exists.
2. **There is no recorder UI yet.** A tray shell exists (M6 S1), but `vrecnu-cli` is still the only way to record or publish. M6 is now fully unblocked.
3. **M8 is blocked on a code-signing certificate**, and no amount of code moves it.
4. **Clipboard delivery is untested on a real shell.** `CliShell` cannot copy — it says so, and
   the engine correctly logs a warning rather than downgrading a successful upload to a failed
   publish. Whether the link actually reaches the clipboard is an M6 question.
5. **`h264_mf` warms up slowly** (~14 fps for the first minute), so short clips finish slightly
   late and play ~0.5 % fast; long ones are exact, because deadlines are absolute. Measured,
   understood, deferred to M9.

---

## Where the documents live

```
ARCHITECTURE.md   what and why — the narrative
AGENTS.md         the rules, and the "do NOT" list
docs/adr/         why each rule exists, and what it cost   ← ADRs win on conflict
HANDOFF.md        the running log: what was measured, and what broke
TESTING.md        what a human still has to check by hand
mindmap.md        this file — the state of the code
docs/oauth-setup.md   the unstarted OAuth registration runbook
```

Read order for anything non-trivial: `ARCHITECTURE.md` → `AGENTS.md` → the relevant ADR.
