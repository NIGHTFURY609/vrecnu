# READ THIS, IDIOT

**You left yourself an escape hatch. This file is here so you remember it exists.**

---

## The short version

Phase 2 uses **Tauri v2**. You reversed [ADR-001](docs/adr/001-phase-2-pure-rust-desktop.md),
which had banned it. You did that on purpose, and you asked for a way back.

**The way back is built into the code, not into your memory.** You do not have to trust that
a future you kept things tidy. There is a second application shell in the repo that has never
been allowed to touch Tauri, and CI fails if it ever does.

---

## What the provision actually is

`apps/desktop/` is a Cargo workspace. Every crate in it is Tauri-free **except one**:

```
crates/
  vrecnu-types/            no tauri
  vrecnu-capture/          no tauri      screen capture
  vrecnu-audio/            no tauri      WASAPI loopback + mic
  vrecnu-camera/           no tauri      the webcam device, opened exactly once
  vrecnu-encode/           no tauri      ffmpeg sidecar
  vrecnu-bubble-win32/     no tauri      the floating webcam bubble (raw Win32)
  vrecnu-publish-server/   no tauri      the loopback server
  vrecnu-engine/           no tauri      ALL the orchestration
  vrecnu-shell-api/        no tauri      the boundary: traits, events, errors

  vrecnu-shell-tauri/      <-- the ONLY crate allowed to name tauri
  vrecnu-cli/              no tauri      headless shell. THIS IS THE PROOF.
  vrecnu-shell-egui/       does not exist yet. Reserved name. Documented contract.
```

`vrecnu-cli` is a real, working, headless version of vrecnu. It records, encodes, and
publishes. It is built and tested in CI on every single commit. **If it still builds, the
escape hatch is still open**, because it links the entire application with zero Tauri in its
dependency graph.

```bash
cargo build -p vrecnu-cli --release
cargo tree -p vrecnu-engine --edges normal | grep -q '^\s*tauri ' && exit 1
```

---

## If you want to go back to pure Rust

You would delete `vrecnu-shell-tauri/` and `apps/desktop/ui/`, and write
`vrecnu-shell-egui/`. That is roughly **2500–4000 lines**, and it implements:

- `ShellServices` — clipboard, open-URL, notify, where `ffmpeg.exe` lives, where data lives
- `PublishSurface` — open the publish URL in the system browser
- a loop that calls `RecorderEngine` and drains its event channel

**Nothing else changes.** Not the capture pipeline, not the audio, not the encoder, not the
publish server, not `packages/publish`. The webcam bubble is already a raw Win32 layered
window with no Tauri in it, so it comes across untouched — that was deliberate.

### The one thing it does NOT cover

**In-app video review.** Under Tauri, playing back the recording you just made is
`<video src="...">` and costs nothing. Under `egui` you have to build a player: ffmpeg decode
→ RGBA → texture upload, plus seeking. Call it another **~1500 lines** and a second ffmpeg
process mode.

If you swap, ship the first `egui` version with "preview = open it in your system player" and
add the scrubber later. Do not let the scrubber block the swap.

---

## Why you'd want to go back

One number: **memory while recording.**

|                    | Native Rust GUI | Tauri / WebView2 |
| ------------------ | --------------- | ---------------- |
| Installer size     | ~12–15 MB       | ~5 MB            |
| RSS while running  | **~30–50 MB**   | **~80–150 MB**   |

A recorder competes for CPU and RAM with the thing it is recording. `AGENTS.md` §1 budgets
**≤150 MB idle RSS** and **0 dropped frames at 1080p30**. Tauri starts near the top of that
budget before you write a line of code.

The mitigation already in place: **vrecnu is a tray app, and the main window is destroyed —
not hidden — when dismissed**, so WebView2's renderer processes exit with it. Idle-in-tray
should land ~15–30 MB.

**Watch M9.** That milestone measures RSS for real. If idle-in-tray is over budget *even with
window destruction*, [ADR-006](docs/adr/006-phase-2-tauri-shell-with-swappable-core.md) says
to reopen the decision rather than reflexively raising the budget.

---

## Things that will quietly kill the escape hatch

If any of these land, the swap stops being cheap. Reject them in review.

1. **`use tauri::...` anywhere outside `vrecnu-shell-tauri`.** Especially `tauri::path` to
   find `ffmpeg.exe` — that is what `ShellServices::ffmpeg_path()` is for.
2. **`AppHandle::emit` called from a capture or audio thread.** Engine threads publish to a
   channel; the shell forwards. Nothing else.
3. **`if (window.__TAURI__)` inside `packages/publish`.** That page is the one artifact Phase 1
   and Phase 2 share ([ADR-002](docs/adr/002-desktop-publish-loopback-origin.md)). The share
   link comes back over `POST /api/complete` on the loopback server precisely so this branch
   never has to exist.
4. **Deleting `vrecnu-cli` because "nobody uses it."** Somebody does. It is the proof.

---

## Where the reasoning is written down properly

- [ADR-006](docs/adr/006-phase-2-tauri-shell-with-swappable-core.md) — the Tauri decision, the
  boundary, the tray mitigation, and the RSS risk.
- [ADR-001](docs/adr/001-phase-2-pure-rust-desktop.md) — why pure Rust was chosen originally.
  Superseded on the shell question only; its point that the hard parts are native regardless
  of shell is still true and is why the reversal was affordable.
- [ADR-005](docs/adr/005-installed-app-not-portable.md) — why there is an installer at all.
- [ADR-007](docs/adr/007-ffmpeg-h264-encoder-and-licensing.md) — unrelated to the shell, but
  it fixed a real contradiction in ADR-004 you had not noticed.
