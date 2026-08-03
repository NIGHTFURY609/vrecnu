# AGENTS.md — vrecnu

Canonical guidance for any AI agent or contributor working in this repository.
Tool-agnostic. `CLAUDE.md` and any other assistant config **defer to this file** — if
they conflict, this file wins. Keep this file updated as the project evolves.

> Read `ARCHITECTURE.md` before doing anything non-trivial. It is the source of truth
> for *what we are building and why*. This file is the source of truth for *how you are
> allowed to build it*.

---

## 1. What this project is

**vrecnu** is an open-source, lightweight screen recorder in the spirit of Loom, with one
defining difference: **recordings upload to the user's own storage (Google Drive first),
never to a proprietary cloud.** Record → auto-upload → shareable link on the clipboard.

Core value props — treat these as invariants, not preferences:

1. **Lightweight.** Small footprint, fast, minimal dependencies. Bloat is a bug.
2. **Portable / no-install.** Web app runs in any browser; desktop ships as a portable
   `.exe` (no installer, no admin rights).
3. **Bring-your-own-storage.** The user owns the data. We are a thin client.
4. **Privacy.** We never touch files we didn't create. No content telemetry.

If a change weakens any of these four, it is wrong by default — stop and flag it.

---

## 2. Delivery phases — DO NOT skip ahead

Work is phased on purpose. **Do not build a later phase's features into an earlier phase.**

- **Phase 1 — Web app (PWA).** Chromium-on-Windows first. Screen + webcam + mic/system
  audio, basic **trim only**, full Drive→link flow. Static-hosted, no backend.
- **Phase 2 — Portable desktop (pure Rust — see ADR-001 in §3).** Native Rust GUI
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
| Encode/edit (desktop) | **FFmpeg as a bundled sidecar** (or `ffmpeg-next` bindings) | LGPL/GPL-compliant build, documented in NOTICE. Bundle at package time; do not commit the binary. |
| Cloud/storage | Google **Drive REST v3** via `fetch` + GIS | No heavyweight Google SDK. Behind the `StorageProvider` interface. **Implemented in the web/TS code only** — the Rust app does not call Drive (see §5). |

Adding a new runtime dependency requires justification against the *lightweight* invariant.
Prefer the platform API or a small utility over a framework. When in doubt, don't add it.

> **ADR-001 (Aug 2026, maintainer-authorized): Phase 2 is a pure-Rust app, not Tauri.**
> The desktop app ships as a native Rust GUI so the portable `.exe` is fully self-contained
> with no WebView2 runtime dependency. Consequence: the "~70% shared React frontend" between
> web and desktop no longer applies — `packages/ui` serves the web app only, and the desktop
> reuses the web side solely through the shared **publish page** it opens in a web view (§5).
> All Google Drive OAuth/upload logic stays in the web/TS code; the Rust app records, edits,
> and hands the local file to that web view. This ADR supersedes the former "Tauri v2 only"
> rule wherever they conflict.

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
- **Phase 2 local-file bridge (desktop → publish web view) — security rules.** To hand the
  local recording to the web view, the Rust app MUST:
  - Serve **only the current recording**, never a directory or any other file.
  - Bind to **loopback only** (`127.0.0.1`) on an ephemeral port, or use a `wry` **custom
    URI scheme** — never a network-facing listener.
  - Gate access with a **one-time capability token** in the URL and restrict **CORS to the
    exact publish-page origin**.
  - **Tear the endpoint down** once publishing completes. A "serve the recordings folder on
    localhost" shortcut is forbidden.

---

## 6. Recording constraints

- **Default output is WebM** (VP9/Opus via `MediaRecorder`). MP4 only via an explicit
  transcode step (cheap in Phase 2 FFmpeg; avoid heavy `ffmpeg.wasm` transcodes in Phase 1).
- **Canvas compositing must cap draw rate to source FPS** to avoid frame drops/desync. If a
  clean composite proves unreliable, record screen and camera as separate tracks and
  composite on export — do not ship a recorder that silently drops frames.
- The live "floating webcam bubble over the whole desktop" is a **Phase 2 desktop-only**
  capability. Do not claim or fake it in the web app; there the bubble is composited into
  the recording and shown in the app's own preview only.
- Never block the UI thread during capture/encode. Heavy work goes to workers (web) or the
  Rust side (desktop).

---

## 7. Privacy, secrets & data

- **No content telemetry, ever.** No recording bytes, frames, audio, filenames, or Drive
  contents leave the user's machine except to the user's own chosen storage.
- Any future analytics must be opt-in, anonymous, and never include recording content.
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
cargo build  --release --manifest-path apps/desktop/Cargo.toml  # portable exe
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
- ❌ Any Drive scope other than `drive.file`.
- ❌ Reading/listing the user's existing Drive files.
- ❌ Routing recording bytes through any vrecnu server — upload is browser→Drive only.
- ❌ Exposing the local file to the web view via anything broader than a single-file,
  loopback-only, one-time-token, CORS-scoped endpoint that is torn down after publish (§5).
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
