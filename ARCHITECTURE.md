# vrecnu — Architecture & Tech Stack Blueprint

> An open-source, lightweight video recoder in the spirit of Loom — but your recordings land in **your own Google Drive**, not someone else's cloud. Record → get a shareable link → done.

*Prepared August 2026. Windows-first, phased delivery, full feature set.*

> **Decision records live in [`docs/adr/`](docs/adr/README.md).** This document is the
> narrative — what we're building and why. `AGENTS.md` holds the rules. The ADRs hold the
> reasoning behind each rule, including the trade-offs we knowingly accepted. Where this
> document and an ADR disagree, **the ADR is newer and wins** — and this document should be
> corrected.

---

## 1. The idea, sharpened

Your instinct is good, and the market proves it: **Cap** (the leading open-source Loom alternative) has essentially validated this exact shape — a beautiful native recorder with instant sharing. Your differentiator is **"bring your own storage."** Instead of paying for a SaaS to host your videos, the app is a thin, free client that pushes to storage *you already own and control*. That's a genuinely compelling open-source pitch: no vendor lock-in, no subscription, no privacy question about "where do my recordings live."

**Positioning in one line:** *Loom's UX, your Google Drive, zero cost.*

> The pitch used to end "…no install." That still describes the **web app**, which is real and permanent — but the desktop app installs (ADR-005), so the tagline shouldn't promise otherwise. The differentiator was always **bring-your-own-storage**, and that is untouched.

A few refinements to the concept before the tech:

- **"Bring your own storage" should be a first-class principle, not just a Drive feature.** Architect the upload layer behind an interface so Drive is the first *provider*, but S3/R2/Dropbox/OneDrive/local-folder can slot in later. This is a small amount of extra design now that massively widens your audience and is very attractive to open-source contributors.
- **The shareable link is the product's magic moment.** Loom's whole growth loop is "stop recording → link is already on your clipboard." Getting that flow tight (auto-upload while you're still reviewing, link ready by the time you finish) matters more than any single recording feature.
- **Don't out-scope Cap on day one.** Cap has a large Rust team. Your edge is *focus + BYO-storage*, not feature parity. Ship a razor-sharp record→Drive→link loop first.

---

## 2. The core architectural decision (and why your phasing is right)

There's one real tension in your brief, and it's worth stating plainly because it drives everything:

| Your goal | Pulls toward |
|---|---|
| "Runs on any PC with nothing to install" | **Browser web app** — nothing to install, ever. This is Phase 1's job, permanently (ADR-005) |
| "Real Loom experience" (webcam bubble floating over your desktop, reliable system audio) | **Native desktop app** — the browser sandbox physically cannot draw over other apps or reliably grab system audio on every OS |

You resolved this correctly by asking for **both, in phases**. That's the right call. The original plan reused a shared React frontend across web and desktop (a Tauri WebView). **Phase 2 keeps that Tauri shell, but strictly as a shell over a native core** (see §2a and [ADR-006](docs/adr/006-phase-2-tauri-shell-with-swappable-core.md)) — so the frontend reuse story mostly survives, while capture, audio and camera are native Rust regardless. The phasing holds either way: Phase 1 proves the record→Drive→link loop, and Phase 2 keeps that loop while replacing the capture layer with native code.

### 2a. Decision: Phase 2 is a Tauri v2 shell over a shell-agnostic core

**Decision ([ADR-006](docs/adr/006-phase-2-tauri-shell-with-swappable-core.md), Aug 2026):** Phase 2 ships as a **Tauri v2 application whose shell is replaceable**. The framework matters less than the structural rule that comes with it:

> **Tauri is an output device and an input device. It is not the application.**

Capture, audio, camera, encode, the publish server and all orchestration live in crates with **zero Tauri dependencies**. Only `apps/desktop/crates/vrecnu-shell-tauri` may name `tauri`, `tao`, `wry` or `webview2-com`, and that boundary is enforced in CI by `apps/desktop/scripts/check-shell-boundary.ps1` — a dependency-graph scan *plus* a link-time proof that the headless `vrecnu-cli` shell still builds. Reverting to a native-Rust `egui` shell would therefore be a UI rewrite, not an application rewrite.

**This reverses an earlier decision, and the history matters.** [ADR-001](docs/adr/001-phase-2-pure-rust-desktop.md) originally banned Tauri, first on portability ("runs off a USB stick"), then — after [ADR-005](docs/adr/005-installed-app-not-portable.md) dropped the portability invariant — on a single remaining number: resident memory while recording (~30–50 MB native vs ~80–150 MB with a WebView). ADR-006 reversed it because:

- **Tauri answers ADR-005's unfinished action items for free** — an NSIS bundler with `installMode: "currentUser"` and a signed static-feed updater, which would otherwise be hand-built.
- **The recorder chrome is the cheap part to get back.** Buttons, source picker and the post-record `<video>` review element come back along with `packages/ui` and Tailwind; the equivalent native work is ~1500 lines of ffmpeg-decode-to-texture.
- **The hard parts do not care.** Native capture, WASAPI audio and the always-on-top bubble are native regardless of shell. Tauri neither helps nor hinders them — which is exactly what makes the reversal affordable.

**What it costs, stated honestly:** the ~30–50 MB RSS advantage is gone, and the ≤ 150 MB idle budget in `AGENTS.md` §1 is genuinely at risk. ADR-006 records the mitigation (a tray app that **destroys** its window on dismiss rather than hiding it) and requires the number to be *measured* in M9 — and the ADR reopened rather than the budget reflexively raised if the mitigation is not enough.

**What this trades away on sharing:** the desktop reuses `packages/ui` for its chrome, but the "~70% shared frontend" figure was never quite the point. The one concrete shared artifact remains the web "connect & upload" page (§4a), which the desktop opens in a web view so the Rust app never implements OAuth or Drive upload at all.

### The phased roadmap

```
Phase 1  ──►  Phase 2  ──────────►  Phase 3
WEB APP       INSTALLED DESKTOP      DEDICATED DESKTOP
(PWA)         (Tauri shell over     (full editor on the
              a native core,        same native core)
              signed per-user
              installer, auto-update)

Proves the      Adds the true         Polish: rich editor,
record→Drive    Loom features         multi-provider storage,
→link loop      browser can't do      effects, viewer page
```

**The no-install promise lives in Phase 1, permanently.** Phase 2 doesn't duplicate it — see [ADR-005](docs/adr/005-installed-app-not-portable.md).

**Phase 1 — Web app (PWA).** Fastest path to a working product. Runs in any Chromium browser on any PC, literally zero install. Delivers screen capture, webcam, mic + system audio (on Chromium/Windows), basic trim, and the full Drive→link flow. This is your MVP and your "any PC" answer.

**Phase 2 — Installed desktop app (Tauri shell, native core).** A **code-signed, per-user installer** — no admin rights — with **auto-update as a first-class feature**. Auto-update is the point: vrecnu is coupled to Google's OAuth and Drive API, and an app you can't update is an app that breaks permanently the next time Google changes something. *(This was originally planned as a portable no-installer exe; [ADR-005](docs/adr/005-installed-app-not-portable.md) records why that changed and what it cost.)* This is where the features the browser *can't* do live: a **real floating webcam bubble** that sits over your whole desktop, rock-solid **system audio** via native Windows loopback, and FFmpeg-powered **brightness/audio adjustment and editing**. The recorder chrome is a Tauri WebView reusing `packages/ui`, while every capture, audio, camera and encode concern is native Rust behind a shell boundary — see §2a for the constraint that keeps the shell replaceable, and §4a for how Google Drive publishing is delegated to a web view so the app itself never implements Drive.

**Phase 3 — Dedicated desktop.** Once the loop is proven: a proper timeline editor, multiple storage backends, cursor smoothing/zoom effects, maybe a lightweight optional viewer page. *(Auto-update moved up to Phase 2 — see ADR-005.)*

---

## 3. Recommended tech stack

### Shared across web + desktop (the storage/publish core)

> Note: this table is the storage/upload/Drive core that both the PWA and the desktop publish web view reuse. The React UI stack below applies to the web app, the publish page, and — since [ADR-006](docs/adr/006-phase-2-tauri-shell-with-swappable-core.md) — the desktop shell's own chrome.

| Concern | Choice | Why |
|---|---|---|
| Language | **TypeScript** (web) / **Rust** (desktop) | TS for the PWA and the shared publish page; Rust for the Phase 2 native app. Both large ecosystems |
| UI framework | **React 19 + Vite** | Ubiquitous, easy for contributors. (Cap uses SolidStart — lighter, but React lowers the contribution barrier for an OSS project) |
| Styling | **Tailwind CSS + shadcn/ui** | Fast, consistent, modern look with minimal effort |
| State | **Component/hook-local state** today; **Zustand** if shared state is ever genuinely needed | The recorder state machine lives in `packages/core` and is driven from a single hook, so nothing needed a store and Zustand is not installed. Add it (tiny, no boilerplate) when state must be shared, not before |
| Drive/upload | **Google Identity Services + Drive REST v3** (plain `fetch`) | No SDK bloat. Lives in the web/TS code (`packages/publish`); the desktop reuses it via the publish web view — so it runs in the browser context in both phases |
| Storage abstraction | **`StorageProvider` interface** (your own) | `GoogleDriveProvider` first; S3/Dropbox/local later |

### Phase 1 — Web app only

| Concern | Choice | Notes |
|---|---|---|
| Screen capture | `navigator.mediaDevices.getDisplayMedia()` | Screen / window / tab |
| Webcam + mic | `getUserMedia()` | Standard |
| System audio | `getDisplayMedia({ audio: true })` + "share system audio" | **Works well on Chromium + Windows**, which is exactly your target. Weak on Firefox/Safari — acceptable for now |
| Compositing (bubble) | **Canvas 2D** draw loop driven by **`requestVideoFrameCallback`** → `canvas.captureStream()`; mix audio via **Web Audio API** | Bakes the webcam bubble into the recorded output. One source frame → one composite; **not** `requestAnimationFrame` (see [ADR-003](docs/adr/003-phase-1-capture-pipeline.md)) |
| Encoding | **`MediaRecorder`**, container by feature detection: **WebM first (VP9 → VP8 → bare WebM), MP4/H.264 last** | Chromium on Windows can record MP4 directly, and ADR-003 §3.3 *prefers* it — but forbids shipping MP4-first until a recorded fragmented MP4 is verified to play **and seek** in Drive's preview player. That check has not been done, so the shipped ladder (`packages/core/src/types.ts`) is WebM-first and MP4 moves up only once the result is recorded in ADR-003 |
| Recording storage | **`timeslice` chunks streamed to OPFS** via `createSyncAccessHandle()` in a Web Worker (IndexedDB fallback) | Flat memory, survives a tab crash, no save prompt before recording, ~10× IndexedDB throughput. Upload streams from the OPFS file. Never accumulate a multi-hundred-MB `Blob` in the tab (ADR-003 amendment) |
| Trim / brightness | Trim via re-mux; brightness/re-encode via **`ffmpeg.wasm`** (lazy-loaded) or **WebCodecs** | Keep heavy editing light in Phase 1 — trim is cheap, deep editing is Phase 2's FFmpeg job |
| Hosting | **Static site** on Cloudflare Pages / GitHub Pages | No backend needed — OAuth is client-side (GIS token client, no secret). Free to host |

### Phase 2 — Installed desktop app (Tauri shell, native core)

Rows marked **shipped** are implemented and measured today; see `HANDOFF.md` for the evidence.

| Concern | Choice | Notes |
|---|---|---|
| Shell / GUI | **Tauri v2**, confined to `vrecnu-shell-tauri` — *in progress (M6, S1 landed)* | [ADR-006](docs/adr/006-phase-2-tauri-shell-with-swappable-core.md) — see §2a. The only crate allowed to name `tauri`/`tao`/`wry`/`webview2-com`, enforced in CI. A tray app that **destroys** its window on dismiss, to defend the RSS budget |
| Second shell (the proof) | **`vrecnu-cli`**, headless — *shipped* | Built in CI on every commit. If it can drive record → encode → publish with no Tauri in its graph, the shell is genuinely swappable |
| Windowing / event loop | **Tauri's own (`tao`)** | The bubble deliberately does *not* use it — a second `winit`/`tao` loop in one process is unsupported |
| Screen capture | **`windows-capture` 2** — *shipped* | Windows Graphics Capture API. `scap` was the other candidate; unchanged by the shell decision either way |
| Webcam bubble | **Raw Win32 layered window** (`vrecnu-bubble-win32`) — *shipped* | *This* is the feature that justifies going desktop. Raw Win32 rather than a second `winit` window, because Win32 message loops are per-thread and so coexist with Tauri's; `UpdateLayeredWindow` also gives true per-pixel alpha. Survives a shell swap untouched |
| Webcam device | **`nokhwa`** (Media Foundation) — *shipped* | Opened **exactly once** and fanned out to both the bubble and the recording; a second open fails on virtual/OEM camera drivers, and it would fail *after* the user pressed record |
| System + mic audio | **WASAPI loopback** via the `wasapi` crate — *shipped* | Reliable full-system audio — no browser checkbox lottery. (`cpal` was the original sketch; `wasapi` is what shipped, for direct loopback control) |
| Encode / mux / edit | **FFmpeg as a bundled sidecar — separate child process only** | **Never linked in** (`ffmpeg-next` and friends are forbidden: MIT project, LGPL/GPL library — see [ADR-004](docs/adr/004-ffmpeg-sidecar-and-size-budgets.md)). Bundled alongside the exe, stripped to what we use. Gives H.264/MP4 (encoder `h264_mf`, not `libx264` — [ADR-007](docs/adr/007-ffmpeg-h264-encoder-and-licensing.md)), `eq` filter for **brightness/contrast/saturation**, `loudnorm`/`volume` for **audio adjust**, and trim/cut for editing — your whole "adjust video/audio" wishlist is FFmpeg filters |
| Publish page host | **A web view owned by the shell** (`vrecnu-shell-tauri`, the only crate allowed to name `wry`) *or* the system default browser (what the headless `vrecnu-cli` shell does today) | Opens the bundled "connect & upload" page from `http://127.0.0.1:<registered port>` **only at publish time** (see §4a / ADR-002). The engine and the publish server never name a web-view crate; they only serve the page. **Not** a custom URI scheme — Google rejects it as an OAuth origin |
| Packaging | **`cargo build --release`**, then Tauri's NSIS bundler (`installMode: "currentUser"`) → **code-signed per-user installer** *(not built yet — M8)* | No admin rights. Installs the exe plus a **stripped** FFmpeg (≤ 25 MB). **Auto-update against a static release feed** (ADR-005) — the feed is static metadata and never sees recording content. Budgets in AGENTS.md §1 |

> **On footprint:** this is the cost of ADR-006, and it is not yet paid off. A WebView shell resides at ~80–150 MB against ~30–50 MB for a native-Rust GUI, and a recorder competes for RAM with the app it is recording. The mitigation is a tray app that **destroys** its window on dismiss, so the WebView is not resident while capture runs — but that is a *plan*, not a measurement. AGENTS.md §1 budgets ≤ 150 MB idle RSS and M9 owes a real number; if the mitigation is not enough, ADR-006 says to reopen the ADR rather than raise the budget. For scale, the whole engine including capture, audio and camera links into a **0.93 MB** stripped release exe today — the shell, not the core, is where the footprint risk lives. FFmpeg is a second installed binary — see the LGPL/GPL compliance item in §7 and [ADR-004](docs/adr/004-ffmpeg-sidecar-and-size-budgets.md); it must be a **child process, never linked**.

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

- **Auth flow by phase:** the **web app** uses the **Google Identity Services (GIS) token client** — the token model: a public client with no secret in the bundle, no refresh token, and ~1-hour access tokens. It is deliberately *not* authorization-code + PKCE: Google requires a client secret at the token endpoint for "Web application" clients, so a secretless PKCE flow is not available to a static page. The token getter caches the current token until shortly before it expires and re-requests via GIS after that; the *first* request must come from a user gesture (a click), or popup blockers suppress Google's window. In **Phase 2 the desktop app does not implement OAuth at all** — it delegates the entire connect-and-upload step to a web view running the same web page, so there is exactly one OAuth implementation to maintain (see §4a). The old native **loopback-redirect OAuth** flow is therefore no longer needed. (Do not confuse that with the loopback *HTTP file server* in §4a, which serves the local recording to the web view and has nothing to do with OAuth.)

### 4a. Phase 2 desktop publishing — how the desktop app reaches Drive

The chosen model: **the Rust app records and edits locally; publishing to Google Drive is handled entirely by a web view.** When the user clicks "Publish to Google Drive":

1. The Rust app starts its **loopback HTTP server** on a registered port and opens a **web view** (owned by the shell — `vrecnu-shell-tauri` — or the system default browser) pointing at the bundled "connect & upload" page it serves — the *same* code path the Phase 1 web app uses for Drive.
2. That page runs the **GIS token client** with the **`drive.file`** scope. The user connects **their own** Google Drive (the bring-your-own-credentials path from §"Two credential models" is available here too).
3. **Getting the local recording to the browser** is the one non-obvious piece — a browser page cannot read files off disk. Because the publish page is itself served from the loopback origin (see the decision box below), **the recording is served from that same origin and same server**, so the page fetches it **same-origin** — no CORS grant, no cross-origin token handling. The endpoint serves **only the current recording**, behind a **single-session capability token** in the URL (one capability per recording and publish session — fetched many times, since the upload reads it in Range windows — and invalidated on completion, cancel or timeout; see the clarification in ADR-006), and is torn down when publishing completes.
4. The page performs a **resumable upload** (`uploadType=resumable`) **directly browser→Drive**, sets `permissions.create {role:"reader", type:"anyone"}`, and reads `webViewLink`.
5. The link is handed back to the Rust app over the **same loopback server** — the page `POST`s the result to `/api/complete` on the origin it was served from (ADR-006: not Tauri IPC, so `packages/publish` carries no shell-specific branch and works identically under any shell, including the system browser). The app then copies it to the clipboard and shows the success state.

**Invariant preserved (privacy / thin-client):** the video travels **only** from the user's machine to the user's own Drive. It never passes through any vrecnu-hosted server — your servers, if any, only ever serve a **static** page. The local file endpoint is loopback-only, single-file, and single-session-token-gated.

### Decision: the publish page is served from a loopback HTTP origin (ADR-002)

> This section previously left the page's origin open and recommended **bundling it and serving it via a `wry` custom scheme** (`vrecnu://…`). **That cannot work.** Google requires OAuth Authorized JavaScript Origins to use `https://`, with loopback (`http://localhost`, `http://127.0.0.1`) as the *only* exemption. A custom scheme is not a registrable origin, so the token request fails with `origin_mismatch` and the whole Phase 2 publish path is dead. The option is removed, not deferred. Full reasoning: **[ADR-002](docs/adr/002-desktop-publish-loopback-origin.md)**.

**What we do instead:** the desktop app **bundles the publish page's static assets inside the exe** and serves them over a **loopback HTTP server on `127.0.0.1`**, from a fixed port tried in order from `47821 … 47825`. All five are registered as Authorized JavaScript Origins on the OAuth client. If all five are busy, publishing fails with a clear vrecnu error rather than falling back to an unregistered port.

This keeps the self-contained property the bundled option was chosen for — no dependency on vrecnu hosting, page pinned to a known-good build — while satisfying Google's origin rules. It also **simplifies the security model**: because the page and the recording share one origin, the file fetch is same-origin, and the CORS restriction the old design needed disappears entirely. The remaining hardening (single file, loopback-only, single-session token, teardown after publish) stays fully in force.

The trade-off accepted: the port list becomes part of the app's public contract and must also be registered by users on the bring-your-own-credentials path.

---

## 5. How each feature you asked for maps to tech

| Feature | Phase 1 (web) | Phase 2 (desktop) |
|---|---|---|
| **Screen capture** | `getDisplayMedia` | `windows-capture` (Windows Graphics Capture) |
| **Webcam bubble overlay** | Composited into the *recording* via canvas (visible in app preview, not over other apps) | **True floating bubble** over the whole desktop (transparent always-on-top window) |
| **System + mic audio** | `getDisplayMedia({audio})` + mic, mixed via Web Audio (Chromium/Windows) | Native WASAPI loopback + mic — reliable |
| **Trim / basic editing** | Trim via remux; light cuts | FFmpeg-backed trim/cut, non-destructive |
| **Adjust video brightness** | `ffmpeg.wasm` `eq` filter or WebCodecs (heavier) | FFmpeg `eq=brightness/contrast/saturation` — instant |
| **Adjust audio** | Web Audio gain; loudness limited | FFmpeg `loudnorm` / `volume` / `highpass` denoise |
| **Video capture (webcam-only recording)** | `getUserMedia` → MediaRecorder | Native camera pipeline |

**Reality check on the bubble in Phase 1:** the browser cannot draw a camera bubble *on top of other applications* — that's an OS-level capability. In the web app, the bubble is composited into the recorded video and shown in the app's own preview. The **live floating bubble over your desktop is the single biggest reason Phase 2 exists.** Set expectations accordingly and it becomes a clean upgrade story rather than a limitation.

---

## 6. Suggested repository structure (monorepo: TS web + Rust desktop)

The desktop app is a **Cargo workspace** (`apps/desktop`) living beside the TypeScript workspace: Tauri-free native core crates plus a replaceable shell ([ADR-006](docs/adr/006-phase-2-tauri-shell-with-swappable-core.md)). The one artifact it is guaranteed to share with the web side is the **"connect & upload" publish page**, factored out so both the PWA and the desktop web view load it; the Tauri shell (M6, in progress — S1 landed) will additionally reuse `packages/ui` for its own chrome.

```
vrecnu/
├─ packages/                    # TypeScript / pnpm workspace (web side)
│  ├─ core/            # shared TS: recorder state machine, types (web app)
│  ├─ storage/         # StorageProvider interface + GoogleDriveProvider (drive.file, resumable)
│  ├─ ui/              # shared React components — PWA, publish page, and (from M6) the Tauri shell's chrome
│  ├─ publish/         # the "connect & upload" page (GIS token client + browser→Drive upload).
│  │                   #   consumed by BOTH the PWA and the desktop web view (§4a)
│  └─ web/             # Phase 1 PWA — Vite app, browser capture engine
├─ apps/
│  └─ desktop/         # Phase 2 Cargo workspace (ADR-006): Tauri-free core + a replaceable shell
│     ├─ Cargo.toml
│     ├─ scripts/      # check-shell-boundary.ps1 — the CI gate for ADR-006
│     └─ crates/       # vrecnu-types, -shell-api (the boundary), -capture, -audio,
│                      #   -camera, -encode (ffmpeg sidecar), -namedpipe, -bubble-win32,
│                      #   -publish-server (loopback), -engine (orchestration),
│                      #   -cli (headless shell); vrecnu-shell-tauri (M6, in progress)
├─ LICENSE             # (already present)
└─ README.md
```

Tooling: **pnpm workspaces + Turborepo** for the TypeScript side; **Cargo** for `apps/desktop`. Building Phase 1 builds the `packages/publish` page and the `storage` layer that Phase 2 reuses verbatim through the web view — so the product's critical path (OAuth + Drive upload) is still written and tested exactly once — and, since ADR-006, the `packages/ui` components the Tauri shell will reuse for its own chrome. The desktop build embeds `packages/publish/dist` at compile time, so run `pnpm build` (or `pnpm --filter @vrecnu/publish build`) before building a desktop binary you intend to publish from; see `AGENTS.md` §8.

---

## 7. Key risks & gotchas (learn these before you code)

- **Canvas compositing can drop frames.** Drawing screen + camera to a canvas at record time and recording the canvas stream is the standard browser trick, but under load it desyncs/drops frames. **Decided mitigation (ADR-003):** drive the draw loop from `requestVideoFrameCallback` on the screen video element, so draws are locked to real source frames rather than display refresh. If that still proves unreliable, record screen and camera as **separate tracks** and composite on export. Prototype this early — it's the riskiest part of Phase 1.
- **A recording held in memory is a recording you can lose.** The default `MediaRecorder` pattern buffers chunks in a JS array until stop; half an hour of 1080p is several hundred MB, and 4K for ~20 minutes has been observed to push a tab past 3 GB and get it OOM-killed, losing everything — *after* the user has done the work. Stream chunks to **OPFS** from the start (ADR-003 + its 2026-08-03 amendment). This is the product's worst failure mode and the cheapest one to design out.
- **Disk is not the bottleneck — don't "optimize" it into a server.** 1080p30 is ~625 KB/s against storage that sustains hundreds of MB/s; the encoder costs orders of magnitude more. Any proposal to buffer chunks on a vrecnu-hosted service for "performance" rests on a false premise *and* breaks the privacy invariant, the cost model, and the upload speed all at once. See `AGENTS.md` §7.
- **`MediaRecorder` WebM has no duration and no seek index.** Players report `Infinity` and the scrub bar doesn't work — so the link you just proudly copied opens a video nobody can seek. Patch the duration before upload (~2 KB utility, not a transcode). Moot only if MP4 ever wins the detection ladder — and it does not lead today, because ADR-003 §3.3 forbids shipping MP4-first until Drive's preview player is verified to seek it.
- **GIS access tokens expire in ~1 hour with no refresh token.** A large recording on a slow uplink can outlive its own token mid-upload. Build `GoogleDriveProvider` around a `getAccessToken()` callback so the upload loop can re-auth and continue against the same resumable session URI. Cheap now, structural later.
- **A full FFmpeg build is 70–90 MB.** That's ~6× the Rust exe and quietly blows the installed-footprint budget. Ship a stripped build (≤ 25 MB) and treat FFmpeg's size as a budget line, not an afterthought (ADR-004). Media Foundation, `openh264` and pure-Rust muxers were evaluated as a ~0 MB alternative and rejected as a *replacement* for FFmpeg — FFmpeg's maturity on the path where bugs destroy recordings is worth the 25 MB (ADR-004). ADR-007 then uses Media Foundation *through* FFmpeg (`h264_mf`) as the H.264 encoder.
- **System audio is a Chromium/Windows privilege.** Your target (Windows + Chromium) is the best case, but document that Firefox/Safari users won't get system audio in the web app. Phase 2 removes this caveat entirely.
- **WebM is the shipped default; MP4 is gated on a Drive check.** Chromium can record MP4 directly, but its output is fragmented MP4 and nobody has yet verified that Drive's preview player seeks it, so the ladder leads with WebM (ADR-003 §3.3 — measure, don't assume). WebM plays everywhere modern, but if users want MP4 for compatibility, that's a transcode — cheap in Phase 2 (FFmpeg), heavier in Phase 1 (`ffmpeg.wasm`). Offer MP4 export in Phase 2.
- **Large-file uploads must be resumable.** A 500 MB recording on hotel Wi-Fi will fail a simple upload. Use resumable upload from day one.
- **OAuth "unverified app" screen + 100-user cap** until you complete basic verification. Plan for it; the BYO-credentials option is your pressure valve.
- **FFmpeg licensing is not trivial — it constrains the architecture.** vrecnu is MIT; FFmpeg is LGPL at best and GPL with some components. Running `ffmpeg.exe` as a **separate child process** is aggregation and costs you nothing. **Linking** it in (`ffmpeg-next` et al.) pulls LGPL relinking obligations onto a statically-linked exe, and outright conflicts if any GPL component is present. Sidecar is mandatory, not preferred (ADR-004). Ship `NOTICE` with the version and `configure` line.
- **Don't over-invest in Phase 1 editing.** Trim is enough for the web app. Deep brightness/audio work belongs in Phase 2 where FFmpeg makes it easy — building it twice is wasted effort.
- **The publish web view's local file bridge is a security surface (Phase 2).** The loopback HTTP server that exposes the recording to the browser (a custom scheme is not an option — ADR-002) must serve **only** the current file plus the bundled publish assets, bind to **`127.0.0.1` only**, gate access with a **single-session capability token**, and shut down after publishing. There is **no CORS at all**: the page and the recording share one `127.0.0.1` origin, so the fetch is same-origin — adding a CORS grant means the origins have diverged from ADR-002, so stop and re-read it. A naive "serve the recordings folder on localhost" would let any local page read the user's recordings. Get this right the first time.
- **A WebView shell costs resident memory, and a recorder can least afford it.** This is the live risk from [ADR-006](docs/adr/006-phase-2-tauri-shell-with-swappable-core.md): ~80–150 MB for the WebView against a ≤ 150 MB idle budget, while competing with the app being recorded. The mitigation is a tray app that destroys its window on dismiss; M9 owes the measurement, and the ADR is to be reopened rather than the budget raised if it is not enough.

---

## 8. Concrete next steps (MVP-first)

A tight Phase 1 MVP you could aim at:

1. **Scaffold the monorepo** (pnpm + Turborepo) with `web`, `ui`, `storage`, `core`.
2. **Recorder spike:** get `getDisplayMedia` + `getUserMedia` → canvas composite → `MediaRecorder` → playable file working. *Prove three things here before any UI work:* the `requestVideoFrameCallback` composite holds frame pacing under load, chunks stream to disk with flat memory, and the output has a valid duration. These are the load-bearing risks (ADR-003).
3. **Google Cloud project:** OAuth client with `drive.file` scope; wire GIS token client in the browser. Register the Phase 2 loopback origins (`http://127.0.0.1:47821`…`:47825`) at the same time — cheap now, annoying later (ADR-002).
4. **Upload + link:** resumable upload → set anyone-with-link → auto-copy `webViewLink`.
5. **Minimal UI:** big record button, source picker, live preview, post-record trim, "Copy link."
6. **Ship the PWA** to Cloudflare/GitHub Pages. That's a usable product.

Then Phase 2, which is under way — `HANDOFF.md` tracks it milestone by milestone. The native core came first and is done: the Cargo workspace and its shell boundary, the always-on-top bubble window, screen capture → FFmpeg → seekable MP4, WASAPI audio, the camera, and the loopback publish server (§4a, M5). What remains is the Tauri shell and its UI (M6, in progress — S1, the tray app, has landed), FFmpeg-backed editing, the signed installer, and the budget measurements ADR-006 owes.

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
