# M6 — the Tauri shell (`vrecnu-shell-tauri`)

**Status:** draft design, awaiting maintainer approval (2026-10-09). Nothing below is
implemented yet. Building any of it needs the MSVC toolchain + Windows SDK, which the dev
machine lacks as of this date (see `HANDOFF.md`).
**Scope:** HANDOFF.md's M6 only — a tray application that drives the existing `Session`
(`RecorderEngine`) through record → stop → review → publish → link on the clipboard, with the
smallest UI that makes vrecnu usable without a terminal.

Everything below assumes as context: [ADR-006](../../adr/006-phase-2-tauri-shell-with-swappable-core.md)
(Tauri is an output and input device, not the application), `vrecnu-shell-api` (the four
boundary traits and `EngineEvent`), `vrecnu-engine::session` (`Session`, `spawn`,
`begin_publish`, the publish pump that already calls `set_clipboard_text`), and
`vrecnu-cli/src/shell.rs` (`CliShell`, the other implementation of `ShellServices`).

---

## Why this shape

The engine was written for this milestone. `Session` already implements every
`RecorderEngine` method a shell needs, `snapshot()` exists so a window that is created late
or reloads can recover from state rather than replaying events, and the publish flow already
emits `PublishOpened`/`PublishProgress`/`PublishDone` and copies the link through
`ShellServices`. M6 is therefore almost entirely *shell*: implement `ShellServices` and
`PublishSurface` for real, forward events to a webview, and build the recorder chrome.

The one structural rule M6 must not bend is ADR-006's: every line that names `tauri`, `tao`,
`wry` or `webview2-com` lives in `vrecnu-shell-tauri`, and the engine never learns a shell
exists. The three leak paths ADR-006 names each have a fixed answer here:

| Leak | M6 answer |
|---|---|
| `tauri::path` in the engine to find `ffmpeg.exe` | `TauriShell::ffmpeg_path()` / `data_dir()` resolve paths shell-side |
| `AppHandle::emit` from a capture thread | A shell-owned pump thread drains `Session::subscribe()` and emits; engine threads never see an `AppHandle` |
| `window.__TAURI__` in `packages/publish` | Not touched. The link returns over `POST /api/complete`, as it does for `vrecnu-cli` |

## Non-goals (M6)

- Editing UI (M7), installer/updater/signing (M8), RSS measurement (M9). M6 must not
  *preclude* M9: the window is destroyed, not hidden.
- Source thumbnails and per-device audio selection — the engine returns "not available yet"
  for both, and the UI shows that honestly rather than faking a picker.
- A global hotkey, multi-monitor polish, an IPC result channel (`provides_ipc_result_channel`
  stays `false`, per ADR-006).

## Layout

```
apps/desktop/crates/vrecnu-shell-tauri/   the ONLY crate that may name tauri/tao/wry/webview2-com
  Cargo.toml                 tauri 2 (tray-icon, protocol-asset), tauri-build, plugins below — exact versions
  build.rs                   tauri_build::build()
  tauri.conf.json            no window at startup; frontendDist -> packages/desktop-ui/dist
  capabilities/default.json  least privilege: our commands + event listen, nothing else
  icons/
  src/main.rs                Builder, tray, window lifecycle, RunEvent::ExitRequested
  src/services.rs            TauriShell: ShellServices
  src/publish.rs             BrowserPublishSurface: PublishSurface
  src/events.rs              the event pump
  src/commands.rs            #[tauri::command] wrappers over Arc<Session>

packages/desktop-ui/          new pnpm package: Vite + React 19 + @vrecnu/ui + Tailwind
  src/engine.ts              TS mirror of the vrecnu-shell-api serde shapes
  src/state.ts               EngineEvent reducer, seeded from get_snapshot()
  src/App.tsx                Recorder / Recording / Review screens
```

`packages/desktop-ui` rather than a folder inside the crate: `pnpm-workspace.yaml` already
globs `packages/*`, so it gets the same lint/typecheck/test/turbo pipeline for free, and the
Cargo tree stays Rust-only. `@tauri-apps/api` is confined to it the same way `tauri` is
confined to one crate: an ESLint `no-restricted-imports` rule in `eslint.config.js` forbids
`@tauri-apps/*` in every other package, which makes "no `__TAURI__` branch in
`packages/publish`" a lint failure rather than a review comment.

## Decisions

Each is a recommendation; the first two need the maintainer's sign-off before S1.

**D1. The publish page opens in the system browser, not an embedded webview.** The GIS token
client signs in through a popup (`window.open` + `postMessage`). In a Tauri/WebView2 window,
new-window requests need custom handling, and popup semantics are the least-verified path in
the whole product — the Drive round trip has never run once. The system browser is exactly the
path `vrecnu-cli publish` already exercises, it costs no second WebView2 renderer (helping the
M9 RSS budget), and users see a browser they already trust while granting Drive access. An
embedded window remains possible later behind the same `PublishSurface` trait.

**D2. The UI lives in `packages/desktop-ui`** (see Layout). The Phase 1 recorder components
are not reused: they are built on `getDisplayMedia`/`MediaRecorder`, which the desktop app
replaces with the native engine. What returns to the desktop is `packages/ui`, Tailwind and
React, as ADR-006 says.

**D3. Window lifecycle.** No window at startup — tray only. Tray left-click or "Open vrecnu"
creates the main window; closing it **destroys** it (WebView2 renderer processes exit with it,
per ADR-006). `RunEvent::ExitRequested` is prevented unless the user chose "Quit" from the tray.
Starting a recording closes the window too: the tray tooltip/icon shows elapsed time and the
tray menu offers "Stop recording". Stopping reopens the window on the review screen.

**D4. Shell services, all called from Rust (no JS permission granted for any of them):**

| `ShellServices` method | Implementation |
|---|---|
| `set_clipboard_text` | `tauri-plugin-clipboard-manager`, Rust API only |
| `open_external_url`, `reveal_in_file_manager` | `tauri-plugin-opener`, Rust API only |
| `notify` | `tauri-plugin-notification` |
| `ffmpeg_path` | `VRECNU_FFMPEG` → resource dir (M8's bundled sidecar) → next to the exe → `PATH`, mirroring `CliShell::locate_ffmpeg` |
| `data_dir` | `app_local_data_dir()/recordings`, created on first use (`CliShell` uses the cwd) |

**D5. Review uses a `<video>` element** reading the finished MP4 (H.264, which WebView2
plays) through Tauri's asset protocol, scoped to `data_dir` only — not the whole disk. That
needs the `protocol-asset` Cargo feature, `app.security.assetProtocol` enabled with that
scope, and the asset origin in the CSP's `media-src`.

## IPC surface

Commands are thin: each takes `State<Arc<Session>>` and returns `Result<T, AppError>`.
`AppError` is already `Serialize`, so errors cross IPC with their `hint` and `session_lost`
intact and the UI can render "port busy" and "your recording is gone" differently.

| Command | Engine call |
|---|---|
| `get_snapshot` | `snapshot()` — the UI calls this first on every window creation |
| `list_sources`, `list_cameras`, `list_audio_devices` | the matching `RecorderEngine` methods |
| `configure` | `configure(SessionConfig)` |
| `start_recording` | `vrecnu_engine::spawn_session(&session)` (not `start`, per `spawn`'s docs) |
| `stop_recording`, `cancel_recording`, `reset` | `stop` / `cancel` / `reset` |
| `publish` | `begin_publish()`, then `BrowserPublishSurface::open(url)` |
| `abort_publish` | `abort_publish()` |
| `reveal_recording` | `ShellServices::reveal_in_file_manager` on the snapshot's recording |

Events: one Tauri event, `engine-event`, whose payload is `EngineEvent` serialized as-is
(`#[serde(tag = "type")]`). The pump thread exits when the receiver disconnects. Nothing is
per-frame (the engine already coalesces), so IPC volume is a few messages a second.

**Contract test against drift.** A Rust test in `vrecnu-shell-tauri` serializes one of each
`EngineEvent` variant and an `AppState` into `packages/desktop-ui/src/__fixtures__/engine.json`
(checked in, regenerated by the test with an env flag); a vitest test parses that fixture with
`engine.ts`'s types and the reducer. Renaming a field on either side then fails a test instead
of failing silently in the UI.

## Slices

| Slice | Delivers | Verified by |
|---|---|---|
| S1 | Crate + `tauri.conf.json` + capabilities + tray (Open / Quit) + window create/destroy + exit prevention. No engine yet. | Hardware: tray appears; opening/closing the window five times leaves no WebView2 renderer behind (Task Manager); boundary script green |
| S2 | `TauriShell`, `Session` in managed state, event pump, `get_snapshot` + list commands | Unit tests for path resolution and the pump (against a fake emitter trait, no webview); contract fixture |
| S3 | `packages/desktop-ui` recorder screen: source, mic/system toggles, camera + bubble, Record; recording state in the tray; Stop from the tray | vitest reducer tests; hardware: 10 s screen + mic recording that plays |
| S4 | Review (`<video>`), Publish → browser, progress + link from events, clipboard | Hardware: with a registered OAuth client, link lands on the clipboard |
| S5 | CI builds `desktop-ui` before the desktop job and clippies the shell; TESTING.md checklist; docs — including `packages/desktop-ui` in AGENTS.md §4's layout and the `@tauri-apps/*` import rule in §10 | CI green |

## Exit criteria for M6

1. `check-shell-boundary.ps1` still passes, unchanged except that nothing new joins
   `$shellFreeCrates`; `cargo tree -p vrecnu-shell-tauri` is the only graph containing `tauri`.
2. `vrecnu-cli` still builds and its 20 tests still pass.
3. Record → stop → review → publish works from the tray app on real hardware; with a
   registered OAuth client, the share link ends up on the clipboard.
4. Closing the window leaves no WebView2 renderer process running (the M9 RSS measurement
   depends on it).
5. `pnpm typecheck`, `pnpm lint`, `pnpm test` and the Cargo checks in `AGENTS.md` §8 pass.

## Risks

- **Toolchain.** Tauri needs MSVC, the Windows SDK and the WebView2 runtime (Windows 11 ships
  it). The first two are missing on the dev machine as of 2026-10-09; nothing in S1+ can be
  compiled until they are back.
- **RSS** (ADR-006's stated risk) — mitigated by D1 and D3, measured in M9.
- **Exe size.** Tauri adds a few MB on top of the engine and the 237 KB publish page, against
  the ≤15 MB budget in AGENTS.md §1. Measure at the end of S1, not at M9.
- **Tray + `prevent_exit` semantics** in Tauri v2 are the piece most likely to behave
  differently from the docs; S1 exists to find out before anything depends on them.
