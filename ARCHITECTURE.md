# vrecnu — Architecture & Tech Stack Blueprint

> An open-source, lightweight video recoder in the spirit of Loom — but your recordings land in **your own Google Drive**, not someone else's cloud. Record → get a shareable link → done.

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

You resolved this correctly by asking for **both, in phases**. That's the right call. The original plan reused a shared React frontend across web and desktop (a Tauri WebView). **We have since chosen to build Phase 2 as a pure-Rust native app instead** (see §2a) — so the reuse story changes shape, but the phasing still holds: Phase 1 proves the record→Drive→link loop, and Phase 2 keeps that loop while replacing only the capture/UI layer with native Rust.

### 2a. Decision: Phase 2 is a pure-Rust app (supersedes the Tauri plan)

**Decision (authorized by the maintainer, Aug 2026):** Phase 2 ships as a **pure-Rust desktop application** — a native Rust GUI (e.g. `egui`, or `Slint`/`iced` for a more designed look) rather than a Tauri shell wrapping the React frontend. Rationale:

- **Genuinely self-contained portable exe.** A native Rust GUI carries its own renderer, so the main app has **no WebView2 runtime dependency** — a cleaner match for the "runs off a USB stick on any PC" invariant than Tauri's WebView-backed shell.
- **The bubble and capture are already native.** `scap` capture, WASAPI/`cpal` audio, and the transparent always-on-top webcam bubble are Rust/native regardless of shell — so most of Phase 2's hard parts don't change.
- **Drive stays out of Rust entirely.** Publishing is delegated to a web view (see §4a), so the Rust app never implements OAuth or Drive upload. That shrinks the native surface area and keeps all Drive logic in one place (the web code).

**What this trades away — state it honestly:** the original "~70% shared frontend" no longer applies to the desktop app's UI. The recorder UI is rebuilt natively in Rust; the React components in `packages/ui` now serve the **web app only**. Reuse across web and desktop is now *conceptual* (the `StorageProvider`/Drive design, the record→publish→link UX) plus **one concrete shared artifact**: the web "connect & upload" page, which the desktop reuses by opening it in a web view. If keeping a single shared frontend ever becomes more important than a WebView-free exe, `Dioxus` (renders the same Rust UI to web + desktop) is the fallback worth revisiting — but that reintroduces a WebView on desktop.

### The phased roadmap

```
Phase 1  ──►  Phase 2  ──────────►  Phase 3
WEB APP       PORTABLE DESKTOP       DEDICATED DESKTOP
(PWA)         (PURE RUST, 1 .exe)    (pure Rust, full editor)

Proves the      Adds the true         Polish: rich editor,
record→Drive    Loom features         multi-provider storage,
→link loop      browser can't do      auto-update, etc.
```

**Phase 1 — Web app (PWA).** Fastest path to a working product. Runs in any Chromium browser on any PC, literally zero install. Delivers screen capture, webcam, mic + system audio (on Chromium/Windows), basic trim, and the full Drive→link flow. This is your MVP and your "any PC" answer.

**Phase 2 — Portable desktop (pure Rust).** A single self-contained `.exe` with no installer — runs from a USB stick, no admin rights, and (with a native Rust GUI) **no WebView2 runtime dependency for the main app**. This is where the features the browser *can't* do live: a **real floating webcam bubble** that sits over your whole desktop, rock-solid **system audio** via native Windows loopback, and FFmpeg-powered **brightness/audio adjustment and editing**. The recorder UI is rebuilt natively in Rust rather than reusing the Phase 1 React frontend — see §2a for what that trades away and §4a for how Google Drive publishing is delegated back to a web view so the app itself never implements Drive.

**Phase 3 — Dedicated desktop.** Once the loop is proven: a proper timeline editor, multiple storage backends, cursor smoothing/zoom effects, auto-update, maybe a lightweight optional viewer page.

---

## 3. Recommended tech stack

### Shared across web + desktop (the storage/publish core)

> Note: under the pure-Rust decision (§2a) the *frontend* is no longer shared — this table is the storage/upload/Drive core that both the PWA and the desktop publish web view reuse. The React UI stack below applies to the web app (and the publish page).

| Concern | Choice | Why |
|---|---|---|
| Language | **TypeScript** (web) / **Rust** (desktop) | TS for the PWA and the shared publish page; Rust for the Phase 2 native app. Both large ecosystems |
| UI framework | **React 19 + Vite** | Ubiquitous, easy for contributors. (Cap uses SolidStart — lighter, but React lowers the contribution barrier for an OSS project) |
| Styling | **Tailwind CSS + shadcn/ui** | Fast, consistent, modern look with minimal effort |
| State | **Zustand** | Tiny, no boilerplate — good for recorder state machine |
| Drive/upload | **Google Identity Services + Drive REST v3** (plain `fetch`) | No SDK bloat. Lives in the web/TS code (`packages/publish`); the desktop reuses it via the publish web view — so it runs in the browser context in both phases |
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

### Phase 2 — Portable desktop (pure Rust)

| Concern | Choice | Notes |
|---|---|---|
| Shell / GUI | **Native Rust GUI — `egui`** (default), or `Slint`/`iced` | Immediate-mode, tiny, statically linked into the exe. **No WebView2 dependency for the main app.** Replaces Tauri. `egui` is the pragmatic pick for a recorder tool; `Slint` if you want a more designed UI |
| Windowing / event loop | **`winit`** | Cross-platform window + input; also hosts the transparent bubble window |
| Screen capture | **`scap`** crate (or `windows-capture`) | Native Windows Graphics Capture API — high-FPS, low overhead. Unchanged by the shell decision |
| Webcam bubble | **Separate always-on-top transparent `winit` window** | *This* is the feature that justifies going desktop — a live camera bubble floating over every app. Native, no WebView |
| System + mic audio | **WASAPI loopback** via `cpal` / native | Reliable full-system audio capture — no browser checkbox lottery |
| Encode / mux / edit | **FFmpeg as a bundled sidecar binary** (or `ffmpeg`/`ffmpeg-next` bindings) | Bundled alongside the exe. Gives H.264/MP4, `eq` filter for **brightness/contrast/saturation**, `loudnorm`/`volume` for **audio adjust**, and trim/cut for editing — your whole "adjust video/audio" wishlist is FFmpeg filters |
| Publish web view | **`wry`** (standalone WebView crate) *or* the system default browser | Opens the web "connect & upload" page **only at publish time** (see §4a). If embedded via `wry`, this one step uses WebView2; if you shell out to the default browser, the app has zero WebView dependency at all |
| Packaging | **`cargo build --release`** → single exe | No installer. Ship FFmpeg as a second bundled binary; "portable" = a small folder (exe + ffmpeg.exe) or a self-extracting single file — genuinely no-install and USB-stick-friendly |

> **On "single portable .exe":** a native-Rust-GUI exe is fully self-contained — it does **not** depend on WebView2 for its own UI, which is a stronger portability story than the old Tauri plan. The only WebView touchpoint is the optional publish step (§4a), and even that disappears if you open the user's default browser instead of embedding `wry`. FFmpeg remains a second bundled binary. Note the LGPL/GPL FFmpeg compliance item in §7.

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

- **Auth flow by phase:** the **web app** uses the **GIS token client / PKCE** flow (no secret in the client). In **Phase 2 the desktop app does not implement OAuth at all** — it delegates the entire connect-and-upload step to a web view running the same web page, so there is exactly one PKCE implementation to maintain (see §4a). The old native **loopback-redirect OAuth** flow is therefore no longer needed. (Do not confuse that with the loopback *HTTP file server* in §4a, which serves the local recording to the web view and has nothing to do with OAuth.)

### 4a. Phase 2 desktop publishing — how a pure-Rust app reaches Drive

The chosen model: **the Rust app records and edits locally; publishing to Google Drive is handled entirely by a web view.** When the user clicks "Publish to Google Drive":

1. The Rust app opens a **web view** (embedded via `wry`, or the system default browser) pointing at the vrecnu web "connect & upload" page — the *same* code path the Phase 1 web app uses for Drive.
2. That page runs **GIS/PKCE OAuth** with the **`drive.file`** scope. The user connects **their own** Google Drive (the bring-your-own-credentials path from §"Two credential models" is available here too).
3. **Getting the local recording to the browser** is the one non-obvious piece — a browser page cannot read files off disk. The Rust app exposes *only the current recording* to the web view through a tightly scoped local channel:
   - **Preferred (embedded `wry`): a custom URI-scheme handler** (e.g. `vrecnu://recording/<id>`) that streams the one file to the page — no open TCP port.
   - **Alternative (default-browser or `wry`): a loopback HTTP server** bound to `127.0.0.1` on a random ephemeral port, serving that single file behind a **one-time capability token** in the URL, with CORS restricted to the web page's exact origin. The server serves only that file and shuts down when publishing completes.
4. The page performs a **resumable upload** (`uploadType=resumable`) **directly browser→Drive**, sets `permissions.create {role:"reader", type:"anyone"}`, and reads `webViewLink`.
5. The link is handed back to the Rust app — via **`wry` IPC** (embedded) or a small **loopback callback** (default browser) — which copies it to the clipboard and shows the success state.

**Invariant preserved (privacy / thin-client):** the video travels **only** from the user's machine to the user's own Drive. It never passes through any vrecnu-hosted server — your servers, if any, only ever serve a **static** page. The local file endpoint is loopback-only, single-file, and one-time-token-gated.

**Open sub-decision (not blocking):** load the publish page from your **hosted** origin (always current, needs the user online — which they must be to upload anyway) **or bundle** the page's static assets inside the exe and serve them via the `wry` custom scheme (fully self-contained, no dependency on your hosting being up). The bundled option is the better fit for the "portable, self-contained" invariant; the hosted option is easier to keep in lockstep with the web app. Recommend **bundled**, with the page pinned to a known-good build.

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

## 6. Suggested repository structure (monorepo: TS web + Rust desktop)

The desktop app is now a **Rust crate (Cargo)** living beside the TypeScript workspace, not a Tauri sub-app that imports `packages/ui`. The one artifact it shares with the web side is the **"connect & upload" publish page**, factored out so both the PWA and the desktop web view load it.

```
vrecnu/
├─ packages/                    # TypeScript / pnpm workspace (web side)
│  ├─ core/            # shared TS: recorder state machine, types (web app)
│  ├─ storage/         # StorageProvider interface + GoogleDriveProvider (drive.file, PKCE, resumable)
│  ├─ ui/              # shared React components — WEB APP ONLY now
│  ├─ publish/         # the "connect & upload" page (PKCE + browser→Drive upload).
│  │                   #   consumed by BOTH the PWA and the desktop web view (§4a)
│  └─ web/             # Phase 1 PWA — Vite app, browser capture engine
├─ apps/
│  └─ desktop/         # Phase 2 PURE RUST app (Cargo crate — NOT Tauri)
│     ├─ Cargo.toml
│     └─ src/          # egui/winit UI, scap capture, cpal/WASAPI audio,
│                      #   ffmpeg sidecar, wry publish web view + loopback file server
├─ LICENSE             # (already present)
└─ README.md
```

Tooling: **pnpm workspaces + Turborepo** for the TypeScript side; **Cargo** for `apps/desktop`. The efficiency argument is narrower than before: building Phase 1 no longer "builds most of Phase 2's UI," but it *does* build the `packages/publish` page and the `storage` layer that Phase 2 reuses verbatim through the web view — so the product's critical path (OAuth + Drive upload) is still written and tested exactly once.

---

## 7. Key risks & gotchas (learn these before you code)

- **Canvas compositing can drop frames.** Drawing screen + camera to a canvas at record time and recording the canvas stream is the standard browser trick, but under load it desyncs/drops frames. Mitigations: cap the canvas draw to the source FPS, or record screen and camera as **separate tracks** and composite on export. Prototype this early — it's the riskiest part of Phase 1.
- **System audio is a Chromium/Windows privilege.** Your target (Windows + Chromium) is the best case, but document that Firefox/Safari users won't get system audio in the web app. Phase 2 removes this caveat entirely.
- **MediaRecorder gives you WebM, not MP4.** WebM plays everywhere modern, but if users want MP4 for compatibility, that's a transcode — cheap in Phase 2 (FFmpeg), heavier in Phase 1 (`ffmpeg.wasm`). Default to WebM; offer MP4 export in Phase 2.
- **Large-file uploads must be resumable.** A 500 MB recording on hotel Wi-Fi will fail a simple upload. Use resumable upload from day one.
- **OAuth "unverified app" screen + 100-user cap** until you complete basic verification. Plan for it; the BYO-credentials option is your pressure valve.
- **FFmpeg licensing.** Ship an LGPL/GPL-compliant build and note it — trivial but don't skip it for an OSS release.
- **Don't over-invest in Phase 1 editing.** Trim is enough for the web app. Deep brightness/audio work belongs in Phase 2 where FFmpeg makes it easy — building it twice is wasted effort.
- **The publish web view's local file bridge is a security surface (Phase 2).** Whatever exposes the recording to the browser — loopback HTTP server or `wry` custom scheme — must serve **only** the current file, bind to **loopback only**, gate access with a **one-time capability token**, restrict **CORS to the exact page origin**, and shut down after publishing. A naive "serve the recordings folder on localhost" would let any local page read the user's recordings. Get this right the first time.
- **Native Rust GUI has a smaller ecosystem than the web.** `egui`/`Slint` are solid for a tool like this, but you lose Tailwind/shadcn and the React component pool. Budget time for building the recorder UI natively, and keep it deliberately simple — the polished timeline editor is Phase 3.

---

## 8. Concrete next steps (MVP-first)

A tight Phase 1 MVP you could aim at:

1. **Scaffold the monorepo** (pnpm + Turborepo) with `web`, `ui`, `storage`, `core`.
2. **Recorder spike:** get `getDisplayMedia` + `getUserMedia` → canvas composite → `MediaRecorder` → downloadable WebM working. *Prove the frame-drop question here.*
3. **Google Cloud project:** OAuth client with `drive.file` scope; wire GIS token client in the browser.
4. **Upload + link:** resumable upload → set anyone-with-link → auto-copy `webViewLink`.
5. **Minimal UI:** big record button, source picker, live preview, post-record trim, "Copy link."
6. **Ship the PWA** to Cloudflare/GitHub Pages. That's a usable product.

Then Phase 2 (pure Rust): stand up the `apps/desktop` Cargo crate with an `egui`/`winit` UI, add the transparent always-on-top floating bubble window, wire in native capture (`scap`) + audio (`cpal`) + FFmpeg, and implement the publish step by opening the `packages/publish` page in a web view with the loopback/`wry` file bridge from §4a. Factor `packages/publish` out of the Phase 1 web app early so it's ready to reuse.
                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                
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
