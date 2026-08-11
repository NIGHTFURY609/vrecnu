# vrecnu — what to test by hand

Everything here needs a human, a real screen, or hardware. Automated checks are already
covered by `pnpm test` / `cargo test` and by CI.

Work top-down: **§1 is the gate for all Phase 2 work**, and §2 catches the Phase 1 regressions
that matter most.

---

## 0. Setup

```powershell
# TypeScript side
pnpm install
pnpm dev                    # PWA on http://localhost:5173

# Rust side (PowerShell, NOT Git Bash - see HANDOFF.md)
cd apps/desktop
cargo test --workspace      # add -j 2 if it OOMs
```

---

## 1. The bubble spike — **do this first, it gates M2 onward**

```powershell
cd apps/desktop
cargo run -p vrecnu-bubble-win32 --example bubble_spike
```

A circular window with a moving colour field and a rotating white square appears at (120,120).
Ctrl+C to exit.

| # | Check | Pass |
|---|---|---|
| 1.1 | Maximize Chrome. Is the bubble still visible on top? | yes |
| 1.2 | Maximize VS Code. Still on top? | yes |
| 1.3 | Play a **borderless-fullscreen** video (YouTube fullscreen). Still on top? | yes |
| 1.4 | Click the bubble, then type in another window. Did focus move? | **no focus loss** |
| 1.5 | Drag the bubble around. | moves smoothly |
| 1.6 | Task Manager → CPU for the spike process | ≤ 3% of one core |
| 1.7 | Task Manager → Memory | ≤ 15 MB |
| 1.8 | Look at the corners of the bubble | transparent, **no black box** |
| 1.9 | Look at the circle's edge | smooth, no stair-stepping |
| 1.10 | Are the colours right (not red/blue swapped)? | correct |
| 1.11 | Drag between a 100% and a 150% DPI monitor | no corruption |
| 1.12 | Ctrl+C | window disappears, clean exit |

**1.4 and 1.3 are the ones that matter.** If either fails, the fallback is a transparent Tauri
window fed JPEG frames over raw IPC — but the camera must *still* be opened only once natively.
Record whatever you find in ADR-006 action item 6.

---

## 2. Phase 1 PWA

`pnpm dev`, then open Chrome. **Use a Chromium browser** — the duration fix and trim are
WebM-only, and Safari takes the MP4 path.

### 2.1 The bug that used to brick the app

1. Click **Start recording**.
2. When Chrome asks which screen to share, press **Cancel**.

| Check | Pass |
|---|---|
| An error appears naming permission | yes |
| The app returns to the picker (not stuck on "Requesting access…") | yes |
| **Start recording works again without a page reload** | yes |

Before the fix this hung forever with no error and no way back.

### 2.2 Memory stays flat — the AGENTS §10 regression

1. DevTools → Performance monitor → tick **JS heap size**.
2. Record screen for **10+ minutes** (longer is better).
3. Watch the heap *during* recording, and especially **at the moment you press Stop**.

| Check | Pass |
|---|---|
| Heap flat during recording | yes |
| **No spike at Stop** | yes — this is the whole point |
| Diagnostics panel shows OPFS bytes climbing while heap does not | yes |

The old code pulled the entire recording into RAM at finalize. A 20-minute recording should
now cost nothing at Stop.

### 2.3 Trim

1. Record ~30 seconds with something visibly changing (a clock, scrolling text).
2. Drag the **Start** and **End** handles inward.
3. **Apply trim**.

| Check | Pass |
|---|---|
| The preview updates to the trimmed range | yes |
| Reported cut points may differ slightly from the handles | expected — cuts snap to keyframes |
| Download it and play in VLC / Windows Media Player | plays |
| **Seek to the middle** | works |
| Duration is roughly what you selected | yes |
| **Undo trim** restores the full recording | yes |

Trim is a lossless container cut, so it should be near-instant even on a large file. If it
takes seconds, something is re-encoding and that is a bug.

### 2.4 Download

| Check | Pass |
|---|---|
| **Save a local copy** opens a real save dialog | yes |
| Cancelling it leaves the app usable | yes |
| The saved file plays and seeks | yes |
| Editing the filename field changes the saved name | yes |
| Illegal characters (`< > : " / \ | ? *`) are stripped, spaces and hyphens survive | yes |

### 2.5 Crash recovery and storage

1. Start recording. Wait ~20 seconds.
2. **Kill the tab** (Task Manager, or close it) — do not press Stop.
3. Reopen the app.

| Check | Pass |
|---|---|
| An amber banner offers the unfinished recording | yes |
| **Recover** plays it back | yes |
| **Discard** removes it, and it does not return after reload | yes |
| After a normal record → **Record again**, DevTools → Application → Storage shows OPFS usage drop | yes |

That last one is the garbage collection. Before the fix, every recording consumed origin quota
permanently and invisibly.

### 2.6 Upload (needs an OAuth client — see HANDOFF.md)

Without a client ID the app runs in **demo mode**.

| Check | Pass |
|---|---|
| Demo mode is clearly labelled | yes |
| The demo link is obviously fake (`example.invalid`) | yes — it must not look like a real Drive link |

With a client ID configured:

| Check | Pass |
|---|---|
| Connect Drive → consent screen requests **only** "see, edit, create and delete only the specific files you use with this app" | yes — anything broader is a scope bug |
| Upload a recording | completes |
| Progress bar advances in steps | yes |
| Link is copied to the clipboard automatically | yes |
| Open the link in a **private window** (not signed in) | video plays |
| **Kill your network mid-upload, restore it, press Try again** | resumes from where it stopped, does not restart |
| Cancel mid-upload | stops cleanly |

The resume test is the important one — it is what AGENTS §5 calls "a defect, not an edge case".

### 2.7 PWA

```powershell
pnpm --filter web build
pnpm --filter web preview     # a service worker needs a production build
```

| Check | Pass |
|---|---|
| Chrome shows an install icon in the address bar | yes |
| Installing it opens a standalone window | yes |
| DevTools → Application → Manifest has no errors | yes |
| DevTools → Application → Service Workers shows one activated | yes |
| Go offline (DevTools → Network → Offline) and reload | app shell still loads |
| Lighthouse → Installable | passes |

### 2.8 Accessibility

| Check | Pass |
|---|---|
| Tab through the whole page — is the focus ring always visible? | yes |
| Every control reachable by keyboard | yes |
| Screen reader (NVDA / Narrator) announces recording start and stop | yes |
| It announces upload progress and completion | yes |
| The device dropdowns announce as "Camera device" / "Microphone device" | yes |

---

## 3. Rust desktop — what can be checked today

M2 onward is not wired up yet, so the CLI reports which milestone owns each missing piece
rather than pretending to work.

```powershell
cd apps/desktop
cargo run -p vrecnu-cli -- doctor
```

| Check | Pass |
|---|---|
| Prints `engine reachable (session state: Idle)` | yes |
| Lists ports 47821–47825 | yes |
| Names M1–M5 for the unimplemented parts | yes |
| `cargo run -p vrecnu-cli -- record --seconds 5` says M2 owns it | yes — an honest refusal, not a silent empty file |

### The boundary gate

```powershell
./scripts/check-shell-boundary.ps1
```

Must print `Shell boundary intact`. **If this ever fails, read `read_this_idiot.md` before
"fixing" it by deleting the check** — it is the only thing keeping the Tauri decision
reversible.

---

## 4. Not yet testable

| Area | Blocked on |
|---|---|
| Screen capture → MP4 | M2 wiring |
| System audio / mic | M3 |
| Camera in the bubble | M4 |
| Desktop publish round-trip | M5 + an OAuth client |
| Desktop UI | M6 |
| Installer, auto-update | M8 + a code-signing certificate |
| RSS / CPU budgets | M9 |
