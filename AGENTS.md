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
- **Phase 2 — Portable desktop (Tauri v2).** Real floating webcam bubble, native system
  audio, FFmpeg-powered editing (brightness/audio/trim). Reuses the Phase 1 frontend.
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
| Desktop shell | **Tauri v2** | **Electron is forbidden.** It violates the lightweight/portable invariant. |
| Native capture | `scap` / `windows-capture` (Rust) | Phase 2+ only. |
| Encode/edit (desktop) | **FFmpeg as a Tauri sidecar** | LGPL/GPL-compliant build, documented in NOTICE. |
| Cloud/storage | Google **Drive REST v3** via `fetch` + GIS | No heavyweight Google SDK. Behind the `StorageProvider` interface. |

Adding a new runtime dependency requires justification against the *lightweight* invariant.
Prefer the platform API or a small utility over a framework. When in doubt, don't add it.

---

## 4. Repository structure & boundaries

```
packages/
  core/      # shared TS: recorder state machine, domain types. PLATFORM-AGNOSTIC.
  storage/   # StorageProvider interface + providers (GoogleDriveProvider first).
  ui/        # shared React components. Browser-safe only.
apps/ or packages/
  web/       # Phase 1 PWA. Browser capture engine lives here.
  desktop/   # Phase 2 Tauri app. Rust in src-tauri/, reuses packages/ui.
```

**Boundary rules (strict):**

- `packages/core` and `packages/storage` MUST NOT import browser-only, Node-only, or
  Tauri-only APIs. They compile and run in both a browser and a Tauri context. Keep them
  pure TS + web-standard APIs (`fetch`, Web Crypto).
- Platform-specific capture code (`getDisplayMedia`, `scap`, WASAPI) lives in the
  `web`/`desktop` apps, **never** in shared packages.
- The recorder and UI must talk to storage **only** through the `StorageProvider`
  interface. No direct `drive.googleapis.com` calls in UI or recording code — Drive is one
  provider implementation, not a hardcoded dependency.

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
- **No client secrets in the repo or shipped web bundle.**
  - Web/PWA: use the GIS token client / **PKCE** flow (public client, no secret).
  - Desktop/Tauri: use the **loopback redirect** (`http://localhost`) native flow.
  - Support a **bring-your-own-credentials** path (user supplies their own OAuth client),
    like rclone.
- **Uploads MUST be resumable** (`uploadType=resumable`). A simple upload that dies on a
  large file over flaky Wi-Fi is a defect, not an edge case.
- **Share flow:** upload → `permissions.create {role:"reader", type:"anyone"}` → copy
  `webViewLink` to clipboard. Do not set broader sharing than "anyone with the link."

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
# Phase 2:
pnpm --filter desktop tauri dev
pnpm --filter desktop tauri build
```

**Verification is required before declaring work done:**

- `pnpm typecheck` and `pnpm lint` must pass. No new type errors, no `// @ts-ignore` without
  a reason comment.
- Add/adjust tests for logic you change (recorder state machine, storage provider, upload
  retry). The Drive upload + link flow is the product's critical path — cover it.
- For recorder/capture changes, verify an actual recording round-trips (record → file →
  playable) before claiming success. Don't assume; check.

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

- ❌ `npm install` / `yarn` — pnpm only.
- ❌ Electron — Tauri v2 only.
- ❌ Any Drive scope other than `drive.file`.
- ❌ Reading/listing the user's existing Drive files.
- ❌ Committing secrets, tokens, or `.env`.
- ❌ Client secrets in web/PWA bundles (use PKCE).
- ❌ Non-resumable uploads.
- ❌ FFmpeg-heavy editing in the Phase 1 web app.
- ❌ Platform-specific imports in `packages/core` / `packages/storage`.
- ❌ Direct Drive API calls outside a `StorageProvider`.
- ❌ Adding dependencies that fight the lightweight invariant without justification.
- ❌ Any content telemetry.
- ❌ Marking work "done" without typecheck + lint + a real functional check.

When a task seems to require breaking one of these, **stop and ask the maintainer** rather
than working around it.
