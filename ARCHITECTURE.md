# vrecnu — Architecture & Tech Stack Blueprint

> An open-source, lightweight screen recorder in the spirit of Loom — but your recordings land in **your own Google Drive**, not someone else's cloud. Record → get a shareable link → done.

*Prepared August 2026. Windows-first, phased delivery, full feature set.*

---

## 1. The idea, sharpened

Your instinct is good, and the market proves it: **Cap** (the leading open-source Loom alternative) has essentially validated this exact shape — a beautiful native recorder with instant sharing. Your differentiator is **"bring your own storage."** Instead of paying for a SaaS to host your videos, the app is a thin, free client that pushes to storage *you already own and control*. That's a genuinely compelling open-source pitch: no vendor lock-in, no subscription, no privacy question about "where do my recordings live."

**Positioning in one line:** *Loom's UX, your Google Drive, zero cost, no install.*

A few refinements to the concept before the tech:

- **"Bring your own storage" should be a first-class principle, not just a Drive feature.** Architect the upload layer behind an interface so Drive is the first *provider*, but S3/R2/Dropbox/OneDrive/local-folder can slot in later. This is a small amount of extra design now that massively widens your audience and is very attractive to open-source contributors.
- **The shareable link is the product's magic moment.** Loom's whole growth loop is "stop recording → link is already on your clipboard." Getting that flow tight (auto-upload while you're still reviewing, link ready by the time you finish) matters more than any single recording feature.
- **Don't out-scope Cap on day one.** Cap has a large Rust team. Your edge is *focus + BYO-storage*, not feature parity. Ship a razor-sharp record→Drive→link loop first.

---

## 2. The core architectural decision (and why your phasing is right)

There's one real tension in your brief, and it's worth stating plainly because it drives everything:

| Your goal | Pulls toward |
|---|---|
| "No install, runs on any PC" | **Browser web app** — nothing to install, ever |
| "Real Loom experience" (webcam bubble floating over your desktop, reliable system audio) | **Native desktop app** — the browser sandbox physically cannot draw over other apps or reliably grab system audio on every OS |

You resolved this correctly by asking for **both, in phases**. That's the right call — and importantly, it's *not* wasted work if you architect a **shared core**. Roughly 70% of the code (UI, Drive integration, sharing, settings, the whole frontend) is identical between the web app and the desktop app. Only the *capture engine* differs. So the plan below is designed so Phase 1 becomes the skeleton of Phase 2, not a throwaway.

### The phased roadmap

```
Phase 1  ──►  Phase 2  ──────────►  Phase 3
WEB APP       PORTABLE DESKTOP       DEDICATED DESKTOP
(PWA)         (Tauri, single .exe)   (Tauri, full editor)

Proves the      Adds the true         Polish: rich editor,
record→Drive    Loom features         multi-provider storage,
→link loop      browser can't do      auto-update, etc.
```

**Phase 1 — Web app (PWA).** Fastest path to a working product. Runs in any Chromium browser on any PC, literally zero install. Delivers screen capture, webcam, mic + system audio (on Chromium/Windows), basic trim, and the full Drive→link flow. This is your MVP and your "any PC" answer.

**Phase 2 — Portable desktop (Tauri).** A single ~10 MB `.exe` with no installer — runs from a USB stick, no admin rights. This is where the features the browser *can't* do live: a **real floating webcam bubble** that sits over your whole desktop, rock-solid **system audio** via native Windows loopback, and FFmpeg-powered **brightness/audio adjustment and editing**. Reuses the Phase 1 UI wholesale.

**Phase 3 — Dedicated desktop.** Once the loop is proven: a proper timeline editor, multiple storage backends, cursor smoothing/zoom effects, auto-update, maybe a lightweight optional viewer page.

---

## 3. Recommended tech stack

### Shared across all phases (the 70%)

| Concern | Choice | Why |
|---|---|---|
| Language | **TypeScript** | One language for web + Tauri frontend; huge ecosystem |
| UI framework | **React 19 + Vite** | Ubiquitous, easy for contributors. (Cap uses SolidStart — lighter, but React lowers the contribution barrier for an OSS project) |
| Styling | **Tailwind CSS + shadcn/ui** | Fast, consistent, modern look with minimal effort |
| State | **Zustand** | Tiny, no boilerplate — good for recorder state machine |
| Drive/upload | **Google Identity Services + Drive REST v3** (plain `fetch`) | No SDK bloat; works identically in browser and Tauri |
| Storage abstraction | **`StorageProvider` interface** (your own) | `GoogleDriveProvider` first; S3/Dropbox/local later |

### Phase 1 — Web app only

| Concern | Choice | Notes |
|---|---|---|
| Screen capture | `navigator.mediaDevices.getDisplayMedia()` | Screen / window / tab |
| Webcam + mic | `getUserMedia()` | Standard |
| System audio | `getDisplayMedia({ audio: true })` + "share system audio" | **Works well on Chromium + Windows**, which is exactly your target. Weak on Firefox/Safari — acceptable for now |
| Compositing (bubble) | **Canvas 2D** draw loop → `canvas.captureStream()`; mix audio via **Web Audio API** | Bakes the webcam bubble into the recorded output |
| Encoding | **`MediaRecorder`** → WebM (VP9/Opus) | Universal. MP4/H.264 output in Chromium is improving but keep WebM as the reliable default |
| Trim / brightness | Trim via re-mux; brightness/re-encode via **`ffmpeg.wasm`** (lazy-loaded) or **WebCodecs** | Keep heavy editing light in Phase 1 — trim is cheap, deep editing is Phase 2's FFmpeg job |
| Hosting | **Static site** on Cloudflare Pages / GitHub Pages | No backend needed — OAuth is client-side (PKCE). Free to host |

### Phase 2 — Portable desktop (Tauri)

| Concern | Choice | Notes |
|---|---|---|
| Shell | **Tauri v2** (Rust core + your existing React UI in a WebView) | ~3–10 MB vs Electron's ~100 MB+. Uses the OS's built-in **WebView2** (present on Win 10/11 by default) |
| Screen capture | **`scap`** crate (or `windows-capture`) | Native Windows Graphics Capture API — high-FPS, low overhead |
| Webcam bubble | **Separate always-on-top transparent Tauri window** | *This* is the feature that justifies going desktop — a live camera bubble floating over every app |
| System + mic audio | **WASAPI loopback** via `cpal` / native | Reliable full-system audio capture — no browser checkbox lottery |
| Encode / mux / edit | **FFmpeg as a Tauri sidecar binary** | Bundled alongside the exe. Gives H.264/MP4, `eq` filter for **brightness/contrast/saturation**, `loudnorm`/`volume` for **audio adjust**, and trim/cut for editing — your whole "adjust video/audio" wishlist is FFmpeg filters |
| Packaging | Tauri **portable/`nsis`-none** build | Produces a no-installer exe. Ship FFmpeg + exe as a small folder, or a single self-extracting exe |

> **On "single portable .exe":** Tauri produces a standalone exe, but it relies on WebView2, which is pre-installed on all current Windows. FFmpeg is a second binary you bundle. So "portable" = a small folder (exe + ffmpeg.exe) or a self-extracting single file — genuinely no-install and USB-stick-friendly, which matches your goal.

---

## 4. Google Drive integration — the important details

This is the heart of your differentiator, so getting the auth model right matters.

### Use the `drive.file` scope — this is the unlock

Google classifies `https://www.googleapis.com/auth/drive.file` as a **non-sensitive scope**. It only grants access to files **your app created**, so:

- It **avoids the restricted-scope security assessment** (the expensive third-party audit that full-`drive`/`drive.readonly` apps must pass).
- Only basic OAuth verification applies — and for the "each user connects their own Drive" model, this is exactly the intended use.
- Users' existing Drive files are never exposed to your app — a strong privacy story for an open-source tool.

Since your app only ever *writes new recordings* to Drive, `drive.file` is a perfect fit. Do **not** request broader scopes; you don't need them and they'd trigger audits.

### Two credential models (offer both — very open-source-friendly)

1. **Shared client ID (default, easy path).** You register one Google Cloud OAuth client and ship its client ID in the app. Users just click "Connect Google Drive." With `drive.file` this needs only basic verification. *Caveat:* an unverified/testing OAuth app caps at 100 test users and shows an "unverified app" screen — fine for early days; complete basic verification before wide release.
2. **Bring-your-own credentials (power-user path, like rclone).** Advanced users paste their *own* Google Cloud OAuth client ID/secret. Zero trust in you, no user cap, no verification dependency on you. This is the classic escape hatch open-source storage tools offer and it removes you as a bottleneck entirely.

### The upload + link flow (the "magic moment")

1. Recording stops (or **starts uploading while the user is still reviewing** — start early for speed).
2. **Resumable upload** to Drive (`uploadType=resumable`) — survives flaky connections, essential for large videos.
3. Set permission: `permissions.create` with `{ role: "reader", type: "anyone" }` → "anyone with the link."
4. Grab `webViewLink` from the file metadata → **copy to clipboard automatically**.
5. Optionally drop the file into a dedicated **"vrecnu Recordings"** folder for tidiness.

- **Auth flow differs slightly by phase:** browser uses **GIS token client / PKCE** (no secret in the client); Tauri desktop uses the **loopback redirect** flow (`localhost` redirect) which is the Google-recommended native-app pattern.

---

## 5. How each feature you asked for maps to tech

| Feature | Phase 1 (web) | Phase 2 (desktop) |
|---|---|---|
| **Screen capture** | `getDisplayMedia` | `scap` native capture |
| **Webcam bubble overlay** | Composited into the *recording* via canvas (visible in app preview, not over other apps) | **True floating bubble** over the whole desktop (transparent always-on-top window) |
| **System + mic audio** | `getDisplayMedia({audio})` + mic, mixed via Web Audio (Chromium/Windows) | Native WASAPI loopback + mic — reliable |
| **Trim / basic editing** | Trim via remux; light cuts | FFmpeg-backed trim/cut, non-destructive |
| **Adjust video brightness** | `ffmpeg.wasm` `eq` filter or WebCodecs (heavier) | FFmpeg `eq=brightness/contrast/saturation` — instant |
| **Adjust audio** | Web Audio gain; loudness limited | FFmpeg `loudnorm` / `volume` / `highpass` denoise |
| **Video capture (webcam-only recording)** | `getUserMedia` → MediaRecorder | Native camera pipeline |

**Reality check on the bubble in Phase 1:** the browser cannot draw a camera bubble *on top of other applications* — that's an OS-level capability. In the web app, the bubble is composited into the recorded video and shown in the app's own preview. The **live floating bubble over your desktop is the single biggest reason Phase 2 exists.** Set expectations accordingly and it becomes a clean upgrade story rather than a limitation.

---

## 6. Suggested repository structure (monorepo, shared core)

```
vrecnu/
├─ packages/
│  ├─ core/            # shared TS: recorder state machine, types
│  ├─ storage/         # StorageProvider interface + GoogleDriveProvider
│  ├─ ui/              # shared React components (record button, preview, settings)
│  └─ web/             # Phase 1 PWA — Vite app, browser capture engine
├─ apps/
│  └─ desktop/         # Phase 2 Tauri app
│     ├─ src/          # imports packages/ui — same frontend
│     └─ src-tauri/    # Rust: scap capture, WASAPI audio, ffmpeg sidecar
├─ LICENSE             # (already present)
└─ README.md
```

Tooling: **pnpm workspaces + Turborepo** (same as Cap). This structure is what makes the phasing efficient — `web` and `desktop/src` both consume `packages/ui` and `packages/storage`, so building Phase 1 *is* building most of Phase 2.

---

## 7. Key risks & gotchas (learn these before you code)

- **Canvas compositing can drop frames.** Drawing screen + camera to a canvas at record time and recording the canvas stream is the standard browser trick, but under load it desyncs/drops frames. Mitigations: cap the canvas draw to the source FPS, or record screen and camera as **separate tracks** and composite on export. Prototype this early — it's the riskiest part of Phase 1.
- **System audio is a Chromium/Windows privilege.** Your target (Windows + Chromium) is the best case, but document that Firefox/Safari users won't get system audio in the web app. Phase 2 removes this caveat entirely.
- **MediaRecorder gives you WebM, not MP4.** WebM plays everywhere modern, but if users want MP4 for compatibility, that's a transcode — cheap in Phase 2 (FFmpeg), heavier in Phase 1 (`ffmpeg.wasm`). Default to WebM; offer MP4 export in Phase 2.
- **Large-file uploads must be resumable.** A 500 MB recording on hotel Wi-Fi will fail a simple upload. Use resumable upload from day one.
- **OAuth "unverified app" screen + 100-user cap** until you complete basic verification. Plan for it; the BYO-credentials option is your pressure valve.
- **FFmpeg licensing.** Ship an LGPL/GPL-compliant build and note it — trivial but don't skip it for an OSS release.
- **Don't over-invest in Phase 1 editing.** Trim is enough for the web app. Deep brightness/audio work belongs in Phase 2 where FFmpeg makes it easy — building it twice is wasted effort.

---

## 8. Concrete next steps (MVP-first)

A tight Phase 1 MVP you could aim at:

1. **Scaffold the monorepo** (pnpm + Turborepo) with `web`, `ui`, `storage`, `core`.
2. **Recorder spike:** get `getDisplayMedia` + `getUserMedia` → canvas composite → `MediaRecorder` → downloadable WebM working. *Prove the frame-drop question here.*
3. **Google Cloud project:** OAuth client with `drive.file` scope; wire GIS token client in the browser.
4. **Upload + link:** resumable upload → set anyone-with-link → auto-copy `webViewLink`.
5. **Minimal UI:** big record button, source picker, live preview, post-record trim, "Copy link."
6. **Ship the PWA** to Cloudflare/GitHub Pages. That's a usable product.

Then Phase 2: wrap the same UI in Tauri, add the floating bubble window, swap in native capture + FFmpeg.

**A good first milestone to commit to:** record a screen + webcam clip in the browser and get a working "anyone with the link" Drive URL on your clipboard. Everything else is refinement on top of that loop.

---

## Sources

- [Cap — GitHub (open-source Loom alternative, Tauri v2 + Rust)](https://github.com/CapSoftware/cap)
- [Open Source Loom Alternatives in 2026 — SendRec](https://sendrec.eu/blog/open-source-loom-alternatives-2026/)
- [Tauri vs Electron 2026: bundle size, RAM, security — PkgPulse](https://www.pkgpulse.com/guides/electron-vs-tauri-2026)
- [Choose Google Drive API scopes (drive.file is non-sensitive)](https://developers.google.com/workspace/drive/api/guides/api-specific-auth)
- [Upload file data — Google Drive resumable uploads](https://developers.google.com/workspace/drive/api/guides/manage-uploads)
- [Screen & webcam recording with getDisplayMedia + getUserMedia — addpipe](https://addpipe.com/get-display-media-with-cam/)
- [Why Canvas Breaks Your Screen Recorder — SendRec](https://sendrec.eu/blog/why-canvas-breaks-your-screen-recorder/)
- [rclone: create your own Google Drive OAuth client_id (BYO-credentials pattern)](https://www.buildwithmatija.com/blog/rclone-google-drive-client-id-oauth-app)
- [scap / screen-recording Rust crates](https://github.com/topics/screen-recording?l=rust)
