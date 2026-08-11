# vrecnu — handoff

**Last updated:** 2026-08-07
**Branch:** `fet0.1`
**Nothing is committed.** All work below is in the working tree.

---

## Verified state, right now

Both of these were observed passing, not assumed:

```
apps/desktop>  cargo test --workspace -j 4     exit 0, 128 tests passed
vrecnu>        pnpm test                        5/5 packages, 135 tests passed
vrecnu>        pnpm lint / typecheck / build    all green
apps/desktop>  cargo clippy -- -D warnings      clean
apps/desktop>  cargo fmt --all -- --check       clean
apps/desktop>  scripts/check-shell-boundary.ps1 pass, incl. link-time proof
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

## In progress: M2 (capture → encode)

**Where it stopped:** `crates/vrecnu-capture/src/wgc.rs` — the Windows Graphics Capture
backend — was just made to compile. It has *not* been run against a real screen.

Done in M2 so far:
- `WgcBackend` implementing `CaptureBackend`: monitor + window enumeration, `start`/`stop`,
  BGRA8 colour format, row-pitch handling.
- Source ids namespaced `monitor:N` / `window:HWND` so the two cannot be confused.
- Pacer, buffer pool, FFmpeg argv builder and `-progress` parser were already done in M0 and
  are unit-tested.

### Next actions, in order

1. **Verify the WGC backend actually delivers frames.** Nothing has exercised it yet. Note
   `Monitor::from_index` is **1-based and rejects 0** — the id namespace must match.
2. **Write the capture→encode wiring** in `vrecnu-engine`: capture callback → pooled buffer →
   pacer tick → writer thread → `ffmpeg` stdin. The pieces exist; nothing joins them.
3. **Spawn the FFmpeg sidecar.** `live_encode_args()` builds the command line already;
   `vrecnu-encode` still needs the `std::process::Command` plumbing, stdin writer thread, and
   stderr `-progress` reader.
4. **Get an `ffmpeg.exe`** to develop against. M8 fetches a stripped build at package time;
   for now any local ffmpeg works via `ShellServices::ffmpeg_path()`.
5. **Implement `vrecnu-cli record`** so M2 is verifiable headlessly. Its done-criteria:
   a 30 s 1080p30 MP4 that plays *and seeks*, `frames_dropped == 0`, RSS flat over 5 minutes,
   and **a static desktop for 10 s producing exactly 300 frames** (that last one is the pacer
   test that catches the silent-wrong-timeline failure).

### Then M3–M9

| M | Scope | Notes |
|---|---|---|
| M3 | WASAPI loopback + mic | `SilenceFiller` already written and tested — loopback delivers *nothing* during silence, same class of bug as the video pacer. Spike the named-pipe audio input early; WAV-then-mux is the documented fallback. |
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
