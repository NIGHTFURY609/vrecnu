# AGENTS.md — vrecnu

Canonical guidance for any AI agent or contributor working in this repository.
Tool-agnostic. `CLAUDE.md` and any other assistant config **defer to this file** — if
they conflict, this file wins. Keep this file updated as the project evolves.

> Read `ARCHITECTURE.md` before doing anything non-trivial. It is the source of truth
> for *what we are building and why*. This file is the source of truth for *how you are
> allowed to build it*. **`docs/adr/` is the source of truth for *why each rule exists*** —
> if a constraint below looks arbitrary or inconvenient, read its ADR before working around
> it. Most of these rules exist to avoid a bug or a legal problem we already found.

---

## 1. What this project is

**vrecnu** is an open-source, lightweight screen recorder in the spirit of Loom, with one
defining difference: **recordings upload to the user's own storage (Google Drive first),
never to a proprietary cloud.** Record → auto-upload → shareable link on the clipboard.

Core value props — treat these as invariants, not preferences:

1. **Lightweight.** Small footprint, fast, minimal dependencies. Bloat is a bug.
2. **A no-install path always exists.** The web app (Phase 1) runs in any browser with
   nothing to install, and that is permanent. The desktop app **installs** — per-user, no
   admin rights, auto-updating, clean uninstall. *(Reworded by
   [ADR-005](docs/adr/005-installed-app-not-portable.md); it previously required a portable
   `.exe`. Auto-update is why — we are coupled to Google's APIs and must be able to ship
   fixes to existing users.)*
3. **Bring-your-own-storage.** The user owns the data. We are a thin client.
4. **Privacy.** We never touch files we didn't create. No content telemetry. **Recording
   bytes never transit vrecnu infrastructure** — not for upload, not for "temporary storage",
   not for a relay. This is the invariant most likely to be eroded by a plausible-sounding
   convenience argument; see §7.

If a change weakens any of these four, it is wrong by default — stop and flag it.

### Budgets — "lightweight" is measurable, not a vibe

Exceeding one of these is a **blocking review finding**, not a discussion. Rationale and
revision policy: [ADR-004](docs/adr/004-ffmpeg-sidecar-and-size-budgets.md).

| Budget | Limit |
|---|---|
| PWA initial JS (gzipped, first load) | ≤ 150 KB |
| PWA total transferred, first load | ≤ 400 KB |
| Desktop exe (release, stripped, no FFmpeg) | ≤ 15 MB |
| Bundled FFmpeg sidecar | ≤ 25 MB |
| Total installed footprint (exe + FFmpeg) | ≤ 45 MB |
| Desktop idle CPU (not recording) | ≤ 2% |
| Desktop idle RSS (not recording) | ≤ 150 MB |
| Recording CPU overhead, 1080p30 | ≤ 10% of one core above baseline capture cost |
| Dropped frames, 1080p30, 5-minute recording | 0 |

These numbers were set before the scaffold existed and **are expected to be revised once
real builds can be measured** — revise them via an ADR amendment with measurements attached,
not by quietly ignoring them.

---

## 2. Delivery phases — DO NOT skip ahead

Work is phased on purpose. **Do not build a later phase's features into an earlier phase.**

- **Phase 1 — Web app (PWA).** Chromium-on-Windows first. Screen + webcam + mic/system
  audio, basic **trim only**, full Drive→link flow. Static-hosted, no backend.
- **Phase 2 — Installed desktop app (pure Rust — see ADR-001 + [ADR-005](docs/adr/005-installed-app-not-portable.md)).** Per-user installer, no admin rights, **auto-update**, code-signed. Native Rust GUI
  (`egui`/`Slint`/`iced`), real floating webcam bubble, native system audio,
  FFmpeg-powered editing (brightness/audio/trim). **Does NOT reuse the Phase 1 React
  frontend** — the recorder UI is native Rust. The one shared artifact is the web
  "connect & upload" page, which the desktop opens in a web view to publish (§5). Google
  Drive OAuth and upload are **not** implemented in the Rust app at all.
- **Phase 3 — Dedicated desktop.** Timeline editor, multi-provider storage, effects,
  auto-update.

**Hard rule:** No FFmpeg / `ffmpeg.wasm`-heavy editing pipeline in Phase 1. Web editing is
limited to trim. Deep brightness/audio work is Phase 2 (native FFmpeg). Building it twice
is forbidden waste.

---

## 3. Tech stack — fixed choices (do not substitute without an ADR)

| Concern | Choice | Constraint |
|---|---|---|
| Language | **TypeScript, strict mode** | No plain `.js`/`.jsx` source. No `any` without an inline justification comment. |
| UI | **React 19 + Vite** | Shared component lib in `packages/ui`. |
| Styling | **Tailwind CSS + shadcn/ui** | No competing CSS-in-JS runtime libs. |
| State | **Zustand** | Do not add Redux/MobX/etc. |
| Package manager | **pnpm** (workspaces) | **NEVER** run `npm install` or `yarn` — it will corrupt the lockfile. Only `pnpm`. |
| Monorepo | **Turborepo** | Respect package boundaries (see §4). |
| Desktop shell / GUI | **Pure-Rust native GUI** (`egui` default; `Slint`/`iced` allowed) | **Electron is forbidden.** **Tauri is no longer used** (see ADR-001) — the main desktop UI is native Rust with **no WebView2 dependency**. A WebView (`wry`) or the system browser is used *only* for the publish step (§5). |
| Desktop windowing | **`winit`** | Hosts the main window and the transparent always-on-top bubble window. |
| Native capture | `scap` / `windows-capture` (Rust) | Phase 2+ only. |
| Encode/edit (desktop) | **FFmpeg as a bundled sidecar — separate child process ONLY** | **Never link FFmpeg into our binary** (`ffmpeg-next`/`ffmpeg-sys`/any bindings are forbidden — we are MIT, FFmpeg is LGPL/GPL; see [ADR-004](docs/adr/004-ffmpeg-sidecar-and-size-budgets.md)). Prefer an LGPL build, stripped to the codecs/filters we use, ≤ 25 MB. Document version + `configure` line in NOTICE. Bundle at package time; do not commit the binary. |
| Cloud/storage | Google **Drive REST v3** via `fetch` + GIS | No heavyweight Google SDK. Behind the `StorageProvider` interface. **Implemented in the web/TS code only** — the Rust app does not call Drive (see §5). |

Adding a new runtime dependency requires justification against the *lightweight* invariant.
Prefer the platform API or a small utility over a framework. When in doubt, don't add it.

> **ADR-001 (Aug 2026, maintainer-authorized): Phase 2 is a pure-Rust app, not Tauri.**
> *(Rationale amended by [ADR-005](docs/adr/005-installed-app-not-portable.md): the decision
> stands, but the reason is now **runtime footprint during capture** — ~30–50 MB RSS native vs
> ~80–150 MB with a WebView — not portability. With an installer, Tauri is actually the
> smaller **download**; it loses on resident memory.)*
> The desktop app ships as a native Rust GUI with no WebView2 runtime dependency.
> Consequence: the "~70% shared React frontend" between
> web and desktop no longer applies — `packages/ui` serves the web app only, and the desktop
> reuses the web side solely through the shared **publish page** it opens in a web view (§5).
> All Google Drive OAuth/upload logic stays in the web/TS code; the Rust app records, edits,
> and hands the local file to that web view. This ADR supersedes the former "Tauri v2 only"
> rule wherever they conflict.
>
> Full text: [ADR-001](docs/adr/001-phase-2-pure-rust-desktop.md).

### Decision records

Every non-obvious rule in this file is backed by an ADR in **`docs/adr/`**. Current set:

| ADR | Decision | Governs |
|---|---|---|
| [001](docs/adr/001-phase-2-pure-rust-desktop.md) | Phase 2 is pure Rust, not Tauri | §2, §3 |
| [002](docs/adr/002-desktop-publish-loopback-origin.md) | Publish page served from a loopback HTTP origin | §5 |
| [003](docs/adr/003-phase-1-capture-pipeline.md) | Chunk-to-disk, container ladder, composite clock | §6 |
| [004](docs/adr/004-ffmpeg-sidecar-and-size-budgets.md) | FFmpeg as a separate process; size/CPU budgets | §1, §3 |
| [005](docs/adr/005-installed-app-not-portable.md) | Installed app + auto-update, not a portable exe | §1, §2 |

Substituting a fixed choice above requires a **new ADR**, not an edit to an existing one.

---

## 4. Repository structure & boundaries

```
packages/                       # TypeScript / pnpm workspace (web side)
  core/      # shared TS: recorder state machine, domain types. PLATFORM-AGNOSTIC.
  storage/   # StorageProvider interface + providers (GoogleDriveProvider first).
  ui/        # shared React components. Browser-safe only. WEB APP ONLY now.
  publish/   # the "connect & upload" page (PKCE + browser→Drive upload).
             #   loaded by BOTH the PWA and the Phase 2 desktop web view.
  web/       # Phase 1 PWA. Browser capture engine lives here.
apps/
  desktop/   # Phase 2 PURE RUST app (Cargo crate — NOT Tauri, NO src-tauri/).
             #   egui/winit UI, scap capture, cpal audio, ffmpeg sidecar,
             #   wry publish web view + loopback file bridge.
```

**Boundary rules (strict):**

- `packages/core` and `packages/storage` MUST NOT import browser-only or Node-only APIs.
  Keep them pure TS + web-standard APIs (`fetch`, Web Crypto) so they run in the browser
  (PWA and desktop web view alike).
- Platform-specific capture code (`getDisplayMedia` in web; `scap`/WASAPI in the Rust
  desktop crate) lives in its own app, **never** in shared packages.
- **The web/UI code talks to storage only through the `StorageProvider` interface.** No
  direct `drive.googleapis.com` calls in UI or recording code — Drive is one provider
  implementation, not a hardcoded dependency.
- **The Rust desktop crate does NOT implement or call Drive at all.** It records, edits,
  and exposes the finished local file to the publish web view (§5). All OAuth and upload
  happen in `packages/publish` running in that web view. Do not add a Drive/OAuth/HTTP-to-
  Google dependency to the Rust crate.

---

## 5. Google Drive & OAuth — security-critical, non-negotiable

These rules protect the privacy invariant and keep us out of Google's audit process. Do
not relax them.

- **Scope: `https://www.googleapis.com/auth/drive.file` ONLY.**
  - NEVER request `drive`, `drive.readonly`, `drive.metadata`, or any other sensitive/
    restricted scope. `drive.file` is non-sensitive, touches only files we create, and
    avoids the restricted-scope security assessment. Requesting more breaks both the
    privacy promise and the "no audit" path.
- **Never read, list, or search the user's pre-existing Drive files.** We only write new
  recordings and manage files we created.
- **No client secrets in the repo or shipped bundles.**
  - Web/PWA **and** the Phase 2 desktop publish web view: use the GIS token client /
    **PKCE** flow (public client, no secret). There is **one** OAuth implementation
    (`packages/publish`); the desktop reuses it by loading that page in a web view. The
    old native **loopback-redirect OAuth** flow is **not** used — do not add it.
  - Support a **bring-your-own-credentials** path (user supplies their own OAuth client),
    like rclone.
- **Uploads MUST be resumable** (`uploadType=resumable`). A simple upload that dies on a
  large file over flaky Wi-Fi is a defect, not an edge case.
- **Share flow:** upload → `permissions.create {role:"reader", type:"anyone"}` → copy
  `webViewLink` to clipboard. Do not set broader sharing than "anyone with the link."
- **The video goes browser→user's Drive ONLY — never through a vrecnu server.** Any hosted
  page we serve is **static**. Recording bytes must not transit our infrastructure (this is
  the privacy/thin-client invariant, and it is content telemetry if violated — see §7).
- **Phase 2 publish origin — see [ADR-002](docs/adr/002-desktop-publish-loopback-origin.md).**
  The desktop app **bundles the publish page's static assets in the exe** and serves them
  over a **loopback HTTP server on `127.0.0.1`**, from a fixed port chosen in order from
  `47821, 47822, 47823, 47824, 47825`. All five are registered as Authorized JavaScript
  Origins on the OAuth client.
  - **A `wry` custom URI scheme (`vrecnu://…`) MUST NOT be used for the publish page.**
    Google requires JavaScript origins to be `https://` with loopback as the only exemption;
    a custom scheme is not a registrable origin and OAuth fails with `origin_mismatch`.
  - If all five ports are occupied, **fail with a clear vrecnu error**. Never fall back to an
    unregistered ephemeral port — that surfaces a confusing Google error instead of ours.
  - Users on the bring-your-own-credentials path must register the same five origins in
    their own Google Cloud project; document this in the BYO setup guide.

- **Phase 2 local-file bridge (desktop → publish web view) — security rules.** The **same**
  loopback server serves the recording, so the page fetches it **same-origin** and no CORS
  grant is involved. The Rust app MUST:
  - Serve **only the current recording** plus the bundled publish assets — never a directory,
    never any other file.
  - Bind to **`127.0.0.1` only** — never a network-facing listener.
  - Gate the recording with a **one-time capability token** in the URL.
  - **Tear the server down** once publishing completes. A "serve the recordings folder on
    localhost" shortcut is forbidden.
  - **Do not add a CORS grant.** If you find yourself needing one, the origins have diverged
    from ADR-002 and something is wrong — stop and re-read it.

---

## 6. Recording constraints

> Rationale for this section: [ADR-003](docs/adr/003-phase-1-capture-pipeline.md).

- **Recorded bytes MUST NOT accumulate in memory.** Run `MediaRecorder` with a `timeslice`
  (start at 2000 ms) and write each `ondataavailable` chunk to the **Origin Private File
  System** — `navigator.storage.getDirectory()` → `createSyncAccessHandle()` **inside a
  dedicated Web Worker** — with **IndexedDB** as the fallback. Memory must stay flat
  regardless of recording length, and an unfinished session must be recoverable after a
  crash. Upload streams from the OPFS file, not from an in-memory `Blob`. A recorder that
  loses 30 minutes of work to a tab crash is a defect, not an edge case.
  - **No save dialog before recording.** OPFS needs no prompt. `showSaveFilePicker()` is for
    the *optional* "save a local copy" action **after** recording, if the user asks.
  - Call `navigator.storage.estimate()` before long recordings and warn on thin headroom;
    call `navigator.storage.persist()` so chunks aren't evicted mid-session.
  - Garbage-collect abandoned OPFS recordings — the user cannot see them and they consume
    quota silently.
  - **Disk is not the bottleneck and never was:** 1080p30 is ~625 KB/s against storage that
    sustains hundreds of MB/s. If someone proposes buffering elsewhere for "performance",
    the premise is wrong — check `AGENTS.md` §7 before entertaining it.
- **Container is chosen by feature detection, first match wins** — this supersedes the old
  "default output is WebM" rule:
  1. `video/mp4;codecs=avc1.42E01E,mp4a.40.2`
  2. `video/webm;codecs=vp9,opus`
  3. `video/webm;codecs=vp8,opus`
  4. `video/webm`

  Gate each with `MediaRecorder.isTypeSupported()`. **The MP4-first ordering is conditional
  on verifying that Chromium's fragmented-MP4 output plays *and seeks* in Google Drive's
  preview player.** If it does not, MP4 drops below WebM. Verify before shipping; do not
  assume.
- **Container metadata MUST be valid before upload.** `MediaRecorder` WebM output omits
  `Info/Duration` and `Cues`, so players report duration `Infinity` and the seek bar breaks —
  which ruins the shared link, i.e. the entire product. Patch the duration with a minimal
  utility (`fix-webm-duration` or equivalent, ~2 KB). This is a metadata patch, **not** a
  transcode, and so does not violate the no-FFmpeg-in-Phase-1 rule in §2.
- **Canvas compositing must be driven by `HTMLVideoElement.requestVideoFrameCallback()`** on
  the screen video element — **not** `requestAnimationFrame` and not `setInterval`. One real
  source frame produces exactly one composite. This is the concrete mechanism for "cap draw
  rate to source FPS." If a clean composite still proves unreliable, record screen and camera
  as separate tracks and composite on export — do not ship a recorder that silently drops
  frames.
- **`GoogleDriveProvider` must take a `getToken(): Promise<string>` callback, not a token
  string.** GIS access tokens last ~1 hour with no refresh token; a large recording on a slow
  uplink can outlive one. The resumable session URI survives re-auth, so the upload loop must
  be able to obtain a fresh token mid-upload.
- The live "floating webcam bubble over the whole desktop" is a **Phase 2 desktop-only**
  capability. Do not claim or fake it in the web app; there the bubble is composited into
  the recording and shown in the app's own preview only.
- Never block the UI thread during capture/encode. Heavy work goes to workers (web) or the
  Rust side (desktop).

---

## 7. Privacy, secrets & data

- **No content telemetry, ever.** No recording bytes, frames, audio, filenames, or Drive
  contents leave the user's machine except to the user's own chosen storage.
- **No vrecnu-hosted storage or relay of recordings — including "temporary" storage.**
  Anything vrecnu hosts serves **static** assets only: the web app, the publish page, and the
  desktop update feed. This rule gets challenged periodically with reasonable-sounding
  framings — buffering chunks server-side "for performance", a relay to "speed up" upload,
  a cache for resumability. **All are rejected**, for four independent reasons, any one of
  which is sufficient:
  1. It destroys the only real differentiator. "Recordings never touch our servers" *is* the
     product; a relay makes vrecnu a worse Loom with no revenue model.
  2. Storage and egress costs scale with adoption — success becomes the thing that kills the
     project.
  3. It makes the maintainer a data controller: GDPR obligations, DMCA exposure, and the
     certainty that someone eventually uploads something illegal.
  4. It is **slower.** user→vrecnu→Drive is two sequential transfers on the user's uplink;
     browser→Drive is one.

  The performance premise is also usually wrong — see §6 on disk throughput. If a case for
  server-side handling of recording bytes still seems compelling, **stop and raise it with
  the maintainer**; it is an architecture change, not an implementation detail.
- Any future analytics must be opt-in, anonymous, and never include recording content.
- The **desktop update feed** (ADR-005) is static release metadata only. It must never
  accept uploads, and must never see recording content.
- **Never commit secrets.** No OAuth secrets, API keys, tokens, or `.env` files. Store user
  credentials/tokens via the OS-appropriate secure store (desktop) or session/PKCE (web).
- Add and respect `.gitignore` for `node_modules/`, `dist/`, `target/`, `.env*`,
  build output, and recordings.

---

## 8. Build, run & verify

> **This repo is pre-scaffold.** Until `package.json`/`pnpm-workspace.yaml` exist, these are
> the *intended* commands — update this section the moment the scaffold lands, and do not
> run them against an empty repo.

```bash
pnpm install            # bootstrap (pnpm ONLY)
pnpm dev                # run the web app (Phase 1)
pnpm --filter web build # build the PWA
pnpm test               # unit tests
pnpm lint && pnpm typecheck
# Phase 2 desktop (pure Rust — Cargo, NOT `tauri` commands):
cargo run    --manifest-path apps/desktop/Cargo.toml   # dev run
cargo build  --release --manifest-path apps/desktop/Cargo.toml  # release exe (then package + sign)
cargo fmt --check && cargo clippy -- -D warnings        # Rust lint/format
cargo test   --manifest-path apps/desktop/Cargo.toml    # Rust unit tests
```

**Verification is required before declaring work done:**

- **Web/TS:** `pnpm typecheck` and `pnpm lint` must pass. No new type errors, no
  `// @ts-ignore` without a reason comment.
- **Rust (desktop):** `cargo clippy -- -D warnings` and `cargo fmt --check` must pass.
- Add/adjust tests for logic you change (recorder state machine, storage provider, upload
  retry). The Drive upload + link flow is the product's critical path — cover it (it lives
  in `packages/publish`).
- For recorder/capture changes, verify an actual recording round-trips (record → file →
  playable) before claiming success. For the Phase 2 publish path, verify the full
  record → open web view → connect Drive → resumable upload → anyone-with-link round-trip.
  Don't assume; check.

---

## 9. Git & contribution conventions

- Branch off `main`; do not commit directly to `main` unless explicitly told to.
- Conventional-commit style messages (`feat:`, `fix:`, `chore:`, `docs:`, `refactor:`).
- Keep PRs scoped to one concern. Update `ARCHITECTURE.md` / this file when a decision
  changes.
- Do not commit generated artifacts, dependencies, or large binaries (bundle FFmpeg at
  build/package time, don't check it in).

---

## 10. Hard "do NOT" list (quick reference)

- ❌ `npm install` / `yarn` — pnpm only (for the TS workspace).
- ❌ Electron. Also ❌ Tauri / any WebView shell for the **main** desktop UI — Phase 2 is a
  pure-Rust native GUI (ADR-001). A web view is allowed **only** for the publish step (§5).
- ❌ Implementing Google Drive / OAuth / upload inside the Rust desktop crate — it lives in
  `packages/publish` and runs in the web view.
- ❌ Serving the publish page from a `wry` custom URI scheme — Google rejects it as an OAuth
  origin (ADR-002). Loopback HTTP on a registered port only.
- ❌ Falling back to an unregistered ephemeral port for the publish page.
- ❌ Adding a CORS grant to the local file bridge — it is same-origin by design (ADR-002).
- ❌ Linking FFmpeg into our binary (`ffmpeg-next`, `ffmpeg-sys`, any bindings) — MIT project,
  LGPL/GPL library; separate child process only (ADR-004).
- ❌ Shipping a stock/full FFmpeg build — stripped, ≤ 25 MB (ADR-004).
- ❌ Accumulating recorded chunks in memory instead of streaming them to OPFS (ADR-003).
- ❌ Prompting for a save location *before* recording starts — OPFS needs no prompt (ADR-003).
- ❌ Storing, buffering, caching, or relaying recording bytes on any vrecnu-hosted service,
  however temporary (§7). Anything we host is static.
- ❌ Shipping the desktop app as a portable exe without auto-update (ADR-005).
- ❌ Uploading a recording whose container has no valid duration (ADR-003).
- ❌ Driving the canvas composite from `requestAnimationFrame` (ADR-003).
- ❌ Exceeding a budget in §1 without an ADR amendment carrying measurements.
- ❌ Any Drive scope other than `drive.file`.
- ❌ Reading/listing the user's existing Drive files.
- ❌ Routing recording bytes through any vrecnu server — upload is browser→Drive only.
- ❌ Exposing the local file to the web view via anything broader than a single-file,
  loopback-only, one-time-token, same-origin endpoint that is torn down after publish (§5).
- ❌ Reintroducing the native loopback-**redirect OAuth** flow (PKCE in the web view now).
- ❌ Committing secrets, tokens, or `.env`.
- ❌ Client secrets in web/PWA bundles (use PKCE).
- ❌ Non-resumable uploads.
- ❌ FFmpeg-heavy editing in the Phase 1 web app.
- ❌ Platform-specific imports in `packages/core` / `packages/storage`.
- ❌ Direct Drive API calls outside a `StorageProvider`.
- ❌ Adding dependencies that fight the lightweight invariant without justification.
- ❌ Any content telemetry.
- ❌ Marking work "done" without typecheck + lint (TS) / clippy + fmt (Rust) + a real
  functional check.

When a task seems to require breaking one of these, **stop and ask the maintainer** rather
than working around it.
