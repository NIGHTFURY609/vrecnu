# ADR-006: Phase 2 uses a Tauri v2 shell over a shell-agnostic core

**Status:** Accepted
**Date:** 2026-08-06
**Deciders:** Maintainer (Jeswin Christie)
**Supersedes:** [ADR-001](001-phase-2-pure-rust-desktop.md) (the shell decision only — see below)

> **Supersession is narrow.** ADR-001 decided *two* things: that Phase 2 is native Rust, and
> that Tauri is forbidden. This ADR reverses the second. Everything ADR-001 says about the
> capture stack being native regardless of shell still holds, and is in fact the reason the
> reversal is affordable.

## Context

ADR-001 rejected Tauri on portability grounds: the WebView2 runtime dependency undermined
"runs off a USB stick." [ADR-005](005-installed-app-not-portable.md) dropped the portability
invariant, and ADR-001 was re-examined rather than grandfathered. It survived on a *second*
argument — runtime footprint during capture, ~30–50 MB RSS native versus ~80–150 MB with a
WebView — with ADR-001 itself noting the reversal in the size argument:

> with an installer, Tauri actually produces a **smaller download** (~5 MB vs ~12–15 MB); it
> loses on resident cost, not on bytes shipped.

So by 2026-08 the decision rested on exactly one number: resident memory while recording.

Three things pushed the other way:

1. **Tauri answers ADR-005's open action items for free.** Action item 4 ("choose installer
   tooling, per-user, no admin") is still unchecked. Tauri ships an NSIS bundler with
   `installMode: "currentUser"` and a signed static-feed updater — precisely what ADR-005
   demands and what would otherwise be hand-built.
2. **The recorder UI is the part ADR-001 called cheap to lose.** Its own trade-off analysis:
   "What is *not* reused is the recorder chrome: buttons, source picker, preview. That is
   rebuildable and deliberately kept simple until Phase 3." A WebView shell gets that chrome
   back — along with `packages/ui`, Tailwind, and the post-record `<video>` review element,
   which is otherwise ~1500 lines of ffmpeg-decode-to-texture work.
3. **The hard parts do not care.** ADR-001, still true: "the *hard* parts of Phase 2 are
   native regardless of shell: `scap` screen capture, WASAPI/`cpal` audio, and a transparent
   always-on-top webcam bubble window. Tauri does not help with any of them." It does not
   hinder them either.

## Decision

**Phase 2 ships as a Tauri v2 application whose shell is replaceable.**

The framework choice is secondary to the structural constraint that comes with it:

> **Tauri is an output device and an input device. It is not the application.**

Every capture, audio, camera, encode, publish-server and orchestration concern lives in
crates with **zero Tauri dependencies**. `apps/desktop/crates/vrecnu-shell-tauri` is the only
crate permitted to name `tauri`, `tao`, `wry`, or `webview2-com`.

### The swappability provision is a link-time proof, not a convention

A second shell — `vrecnu-cli`, headless — is built from day one and kept green in CI forever.
If it can drive a full record → encode → publish cycle with no Tauri in its dependency graph,
then reverting to a pure-Rust `egui` shell is a UI rewrite, not an application rewrite.

```bash
cargo build -p vrecnu-cli --release      # must link with zero tauri in the graph
cargo tree -p vrecnu-engine --edges normal | grep -q '^\s*tauri ' && exit 1
```

"We'll be careful" would not have survived six months of contributions. The three leak paths
are specific and predictable, and each is pre-empted by the design:

| Leak | Pre-empted by |
|---|---|
| Reaching for `tauri::path` in the engine to locate `ffmpeg.exe` | `ShellServices::ffmpeg_path()` / `data_dir()` |
| `AppHandle::emit` called from a capture thread | `EngineEvent` channel + `subscribe()` |
| `if (window.__TAURI__)` inside `packages/publish` | The share link returns over `POST /api/complete` (below) |

`read_this_idiot.md` at the repo root exists so the provision is discoverable by someone who
has never read this file.

### The share link returns over loopback HTTP, not Tauri IPC

`packages/publish` is the *one* artifact Phase 1 and Phase 2 share (ADR-002). A Tauri-shaped
branch inside it would put the shell exactly where ADR-001 and ADR-002 worked hardest to keep
it out, and the `egui` shell — which opens the system browser — has no IPC at all. The page
therefore reports completion to the loopback server it was already served from. The seam
(`PublishSurface::provides_ipc_result_channel`) stays in the API, unimplemented.

### vrecnu is a tray application

The main window is created on demand and **destroyed** — not hidden — when dismissed, so
WebView2's renderer processes exit with it. This is the mitigation for the RSS risk below and
is a genuine design decision, not a workaround: the UI is also unwanted on screen while
recording.

## Consequences

**Easier:**

- ADR-005's installer and auto-update requirements are met by the framework.
- `packages/ui`, Tailwind and React return to the desktop. ADR-001's "the ~70% shared
  frontend claim is void" is itself now void — **`AGENTS.md` §4's "`ui/` — WEB APP ONLY now"
  must be reworded, not silently violated.**
- Post-record review is a `<video>` element instead of a decode-to-texture pipeline.

**Harder:**

- **The ≤150 MB idle RSS budget is at genuine risk.** ADR-005's own table predicts 80–150 MB
  for a WebView shell, and a WebView2 host + renderer + GPU process for one window
  realistically lands at 90–140 MB before any capture buffers. The tray design is expected to
  bring idle-in-tray to ~15–30 MB, but this must be **measured in M9 and the budget amended
  with numbers attached**, per AGENTS.md §1 — not quietly ignored.
- Two runtimes in one process, which ADR-001 listed as a cost and which remains one.
- The swap escape hatch is real and cheap for everything *except* in-app video scrubbing.

**Unchanged:**

- FFmpeg remains a child process, never linked (ADR-004).
- The publish page is still served from `127.0.0.1:47821…47825`, still same-origin, still no
  CORS (ADR-002).
- The Rust side still implements no OAuth and no Drive access whatsoever.

## Clarifications this ADR records

- **The capability token is single-*session*, not single-*request*.** AGENTS.md §5 says
  "one-time capability token"; the recording endpoint is necessarily fetched many times
  (Range windows, `<video>` preview). One capability, scoped to one recording and one publish
  session, invalidated on completion/cancel/timeout. Stated here so a future reviewer reading
  §5 literally does not file it as a violation.
- **`overlay` and `scale` are added to ADR-004 §4.2's filter allow-list.** That list is
  otherwise a closed set; `overlay` is required to burn the webcam bubble into the recording.

## Options Considered

### Option A: Stay pure-Rust `egui` (ADR-001 as written)

**Pros:** lowest RSS; no second runtime; no reversal.
**Cons:** installer and updater must be built by hand while ADR-005's action items stay open;
recorder chrome and video review built from scratch; smallest contributor pool.

### Option B: Tauri v2 with a shell-agnostic core (chosen)

**Pros:** installer + updater + review UI for free; shared frontend returns; the hard native
work is unaffected; the decision is reversible by construction.
**Cons:** RSS risk against a budget that is already the tightest number in ADR-004; two
runtimes; requires the CI gate below to stay honest.

### Option C: Tauri v2 without the boundary

Rejected. It is the same benefit with none of the reversibility, and it makes the RSS risk
permanent rather than provisional.

## To revisit

- **If M9 measurements show idle-in-tray RSS above the budget even with window destruction**,
  reopen this ADR rather than amending the budget upward by reflex.
- ADR-001's `egui` shell is not dead. `vrecnu-shell-egui` is a reserved crate name with a
  documented contract, and the CLI shell keeps the path open.

## Action Items

1. [x] Scaffold `apps/desktop` with the crate boundary and the `vrecnu-cli` shell (M0).
2. [x] Add the CI dep-graph gate and keep `vrecnu-cli` green forever (M0).
3. [x] Write `read_this_idiot.md` at the repo root.
4. [x] Reword `AGENTS.md` §3 (Tauri forbidden), §4 (`src-tauri/`, `ui/` web-only) and §10.
5. [ ] Measure idle-in-tray and recording RSS in M9; amend AGENTS.md §1 with numbers.
6. [ ] Record the bubble spike results (ADR-001 action item 3) here when M1 completes.
