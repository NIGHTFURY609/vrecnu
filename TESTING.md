# vrecnu — what to test by hand

Everything here needs a human, a real screen, or hardware. Automated checks are already
covered by `pnpm test` / `cargo test` and by CI.

Work top-down: **§1 is the gate for all Phase 2 work**, §2 catches the Phase 1 regressions that
matter most, and §3 is the desktop recorder, which now actually records.

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

## 3. Rust desktop — recording works, and here is how to prove it

Needs an `ffmpeg.exe`. Until M8 bundles one, point at any local build:

```powershell
cd apps/desktop
$env:VRECNU_FFMPEG = "C:\ffmpeg\bin\ffmpeg.exe"   # or leave unset to use PATH
cargo build --release -p vrecnu-cli
```

### 3.1 Capabilities

```powershell
cargo run -p vrecnu-cli -- doctor
cargo run -p vrecnu-cli -- sources
```

| Check | Pass |
|---|---|
| `doctor` prints `engine reachable (session state: Idle)` | yes |
| `doctor` lists ports 47821–47825 | yes |
| `doctor` prints `capture backend ready (N sources)` | yes |
| `doctor` prints an `h264 encoder` line naming `h264_mf` or `libopenh264` | yes |
| `doctor` prints `audio backend     mic ready, system ready` (or `unavailable` per device) | yes |
| `doctor` reports a real camera count, and still names M5 for the publish server | yes |
| `sources` lists your displays with `*` on the primary, then windows | yes |
| Display ids are **1-based** (`monitor:1`, never `monitor:0`) | yes |

### 3.2 The pacer test — **run this one first**

This is the check that catches the silent-wrong-timeline failure. Windows Graphics Capture
delivers a frame only when the screen *changes*, so a static desktop must still produce a full
timeline or the recording comes out shorter than the session with nothing reporting an error.

**Leave the screen completely still for the whole ten seconds.**

```powershell
.\target\release\vrecnu-cli.exe record --seconds 10 --out $env:TEMP\pace.mp4
```

| Check | Pass |
|---|---|
| Final line reports **`written 300`** | exactly 300, not 299, not 297 |
| `dropped 0` | yes |
| `frames in` is far *lower* than 300 (a still screen delivers very few) | yes — that is the point |
| `duplicated` makes up the difference | yes — duplication is expected and cheap |

Then confirm the file agrees, rather than trusting our own counter:

```powershell
ffprobe -v error -select_streams v:0 -count_frames `
  -show_entries stream=width,height,avg_frame_rate,nb_read_frames,duration `
  -of default=noprint_wrappers=1 $env:TEMP\pace.mp4
```

Expect `avg_frame_rate=30/1`, `nb_read_frames=300`, `duration=10.000000`.

### 3.3 A real 30-second recording

```powershell
.\target\release\vrecnu-cli.exe record --seconds 30 --out $env:TEMP\clip.mp4
```

| Check | Pass |
|---|---|
| `written 900`, `dropped 0` | yes |
| Plays in VLC / Windows Media Player | yes |
| **Seek to the middle** | works |
| **Colours are right — not red/blue swapped** | yes |
| **No diagonal skew or leaning** | yes — a skew means the row-pitch copy broke |
| Text is legible, not sheared | yes |
| Task Manager → memory for `vrecnu-cli` is flat for the whole run | yes (~73–81 MB observed at 1080p) |

A 5-minute soak (`--seconds 300`) is the stronger version: 9000 frames, 0 dropped, working set
settling at ~80 MB and not moving. One 8 MB step early is expected — it is a single frame
buffer being paged in — but the number must **stop** climbing. If it keeps rising, the pool is
being bypassed somewhere and that is the bug the whole design exists to prevent.

The two visual checks are the ones automation cannot do. A wrong stride or a swapped channel
order produces a file that passes every numeric check and is still wrong.

### 3.4 Audio — mic, system, and both

```powershell
.\target\release\vrecnu-cli.exe record --seconds 5 --mic --out $env:TEMP\mic.mp4
.\target\release\vrecnu-cli.exe record --seconds 5 --system --out $env:TEMP\sys.mp4
.\target\release\vrecnu-cli.exe record --seconds 5 --mic --system --out $env:TEMP\both.mp4
```

For `--system`, play something with clear audio for the whole recording (a YouTube video, a
tone) — WASAPI loopback correctly reports silence when nothing is playing, and a silent
`--system` test proves nothing either way.

| Check | Pass |
|---|---|
| Each file has an AAC stream, `48000` Hz, `2` channels | `ffprobe -show_entries stream=codec_name,sample_rate,channels` |
| The audio isn't silence | `ffmpeg -i <file> -af volumedetect -f null -` → `mean_volume` well above the ~-91dB digital-silence floor (real captures on this session ran -30 to -45dB) |
| Video and audio durations match | `ffprobe -show_entries format=duration` on the whole file, not per-stream |
| The recording still finishes (may take longer than the requested duration) | yes — a few extra seconds during FFmpeg's encoder warm-up is expected, see HANDOFF.md's M2/M3 known-issues; a recording that never finishes is not |

### 3.5 Camera and the floating bubble

```powershell
cargo run -p vrecnu-cli -- cameras
cargo run -p vrecnu-cli -- record --seconds 10 --camera 0 --out cam.mp4
```

| Check | Expect |
|---|---|
| `cameras` lists your webcam(s) with an id and a name | yes |
| The log line `camera opened` names a *small* format (e.g. `320x240 NV12`), not the sensor's maximum | yes — the bubble is ~240 px and the crop throws the rest away |
| `frames_dropped 0` | yes |
| The recording has a circular camera bubble in the **bottom-right**, with a white ring and a soft shadow | yes |
| Skin tones look natural | yes — a blue face means the RGBA→BGRA swap regressed |
| No dark fringe around the circle's edge | yes — a fringe means straight/premultiplied alpha got crossed |

Now the floating window:

```powershell
cargo run -p vrecnu-cli -- record --seconds 10 --camera 0 --bubble --out bub.mp4
```

| Check | Expect |
|---|---|
| A circular camera bubble floats above every other window while recording | yes |
| It disappears when the recording ends, and the webcam light goes out | yes |
| The log says `camera_burned_in=false` | yes — over a monitor capture the window is recorded by the OS, and burning it in as well draws it twice |
| The recording contains **exactly one** bubble, where the window was | yes — two offset circles is the bug in HANDOFF.md M4 §2 coming back |

And everything at once:

```powershell
cargo run -p vrecnu-cli -- record --seconds 10 --camera 0 --bubble --mic --system --out full.mp4
ffprobe -hide_banner full.mp4
```

| Check | Expect |
|---|---|
| Both an h264 and an aac stream | yes |
| `frames_dropped` is non-zero on this one | expected — FFmpeg takes seconds to become ready with an audio input; see HANDOFF.md M4 known-issues |

### 3.6 Things known to be unexercised

Worth a look if you have the hardware:

- **A second monitor.** `--source monitor:2`. Only a single display has been tested.
- **A window rather than a screen.** `--source window:<HWND>` from `sources`. Then resize it
  mid-recording — the run should *stop with a resolution-changed error*, not freeze on the
  last good frame.
- **Recording something that moves constantly** (video playback). Watch `dropped`; it should
  stay at 0, and `encoder fps` tells you whether FFmpeg is keeping up.
- **A USB/Bluetooth mic or headset disconnecting mid-recording.** `AudioError::
  DeviceDisconnected` exists but hot-swap recovery was explicitly scoped out — expect the
  recording to end with an error, not silently continue.

### The boundary gate

```powershell
./scripts/check-shell-boundary.ps1 -ManifestPath (Resolve-Path .\Cargo.toml)
```

Must print `Shell boundary intact`. **If this ever fails, read `read_this_idiot.md` before
"fixing" it by deleting the check** — it is the only thing keeping the Tauri decision
reversible. Pass `-ManifestPath` explicitly: `$PSScriptRoot` comes back empty under some
invocations and the default silently resolves to `\..\Cargo.toml`.

---

## 4. Not yet testable

| Area | Blocked on |
|---|---|
| Desktop publish round-trip | M5 + an OAuth client |
| Desktop UI | M6 |
| Trim / export on the desktop side | M7 (`render_args` is built and tested; nothing calls it) |
| Installer, auto-update | M8 + a code-signing certificate |
| RSS / CPU budgets as *budgets* | M9 — recording RSS is now measurable, idle RSS needs the shell |

---

## M5 — the loopback publish server

Automated coverage is real here (`cargo test -p vrecnu-publish-server` drives an actual server
over an actual socket), so this list is only what a machine cannot check.

1. **The page renders.** `pnpm --filter @vrecnu/publish build`, then
   `cargo run -p vrecnu-cli -- publish <some.mp4>`. A browser should open on
   `http://127.0.0.1:47821/?t=…` and show "Publish to Google Drive" with the file name and size.
   *A blank page means an asset was served with the wrong `Content-Type`* — check the console,
   not the network tab.
2. **The Drive round trip.** Needs a registered OAuth client (`docs/oauth-setup.md`). Connect,
   upload, and confirm the terminal prints a `webViewLink` that actually opens.
3. **All five ports busy.** Occupy 47821–47825 (`python -m http.server` five times will do) and
   run `publish`. It must refuse with an actionable message naming the ports — **never** fall
   back to another port, because Google would then reject the origin with its own error screen.
4. **Close the browser mid-publish.** The CLI should keep waiting; `Ctrl-C` must leave no
   listener behind (`Get-NetTCPConnection -LocalPort 47821 -State Listen`).
5. **Publish twice in a row.** The second run should bind **47821 again**, not 47822. A creeping
   port number means a session is leaking its listener.
