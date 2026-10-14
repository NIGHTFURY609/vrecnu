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

These numbers were set before there were real builds to measure and **are expected to be
revised once they can be** (ADR-006 owes the desktop RSS measurement in M9) — revise them via
an ADR amendment with measurements attached, not by quietly ignoring them.

---

## 2. Delivery phases — DO NOT skip ahead

Work is phased on purpose. **Do not build a later phase's features into an earlier phase.**

- **Phase 1 — Web app (PWA).** Chromium-on-Windows first. Screen + webcam + mic/system
  audio, basic **trim only**, full Drive→link flow. Static-hosted, no backend.
- **Phase 2 — Installed desktop app (Tauri v2 shell over a shell-agnostic pure-Rust core — see [ADR-006](docs/adr/006-phase-2-tauri-shell-with-swappable-core.md), which supersedes ADR-001 on the shell question, + [ADR-005](docs/adr/005-installed-app-not-portable.md)).** Per-user installer, no admin rights, **auto-update**, code-signed. Capture, audio, camera, encode, publish server and orchestration are native Rust crates with
  **zero Tauri dependencies**; the Tauri shell (`vrecnu-shell-tauri`, milestone M6 — **planned,
  not started**) is only the window/tray/UI around them, and reuses `packages/ui` for its
  recorder chrome. Real floating webcam bubble (raw Win32), native system audio,
  FFmpeg-powered editing (brightness/audio/trim). The one artifact shared with the web side
  by construction is the "connect & upload" page, which the desktop opens in a web view (or
  the system browser) to publish (§5). Google Drive OAuth and upload are **not** implemented
  in the Rust app at all.
- **Phase 3 — Dedicated desktop.** Timeline editor, multi-provider storage, effects.
  *(Auto-update moved up to Phase 2 — ADR-005.)*

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
| State | **React state/hooks; Zustand only if shared state is ever needed** | Zustand was removed because nothing imported it — the recorder keeps its state local to `useRecordingEngine`, so **no store is installed today**. Add Zustand (not Redux/MobX/etc.) when state genuinely needs sharing, not before (§3's "when in doubt, don't add it"). |
| Package manager | **pnpm** (workspaces) | **NEVER** run `npm install` or `yarn` — it will corrupt the lockfile. Only `pnpm`. |
| Monorepo | **Turborepo** | Respect package boundaries (see §4). |
| Desktop shell / GUI | **Tauri v2**, over a shell-agnostic core ([ADR-006](docs/adr/006-phase-2-tauri-shell-with-swappable-core.md)) | **Electron is forbidden.** Tauri is permitted again — ADR-006 supersedes ADR-001 on the shell question only. **Every Tauri dependency must live in `vrecnu-shell-tauri` (M6 — planned, the crate does not exist yet) and nowhere else**; the headless `vrecnu-cli` shell is built in CI as a link-time proof of that. `egui`/`Slint`/`iced` remain the documented fallback. |
| Desktop bubble window | **Raw Win32 layered window** (`windows` crate + `tiny-skia`) | Not a second Tauri webview: that would force the camera open twice, which hard-fails on virtual/OEM devices *after* the user hits record. Survives a shell swap unchanged. |
| Native capture | **`windows-capture`** (Windows Graphics Capture); audio via **`wasapi`**; camera via **`nokhwa`** | Phase 2+ only. These are what shipped (`vrecnu-capture`, `vrecnu-audio`, `vrecnu-camera`); `scap`/`cpal` were earlier candidates. |
| Encode/edit (desktop) | **FFmpeg as a bundled sidecar — separate child process ONLY** | **Never link FFmpeg into our binary** (`ffmpeg-next`/`ffmpeg-sys`/any bindings are forbidden — we are MIT, FFmpeg is LGPL/GPL; see [ADR-004](docs/adr/004-ffmpeg-sidecar-and-size-budgets.md)). Prefer an LGPL build, stripped to the codecs/filters we use, ≤ 25 MB. **H.264 encode is `h264_mf`, not `libx264`** — `libx264` requires `--enable-gpl` and cannot exist in an LGPL build ([ADR-007](docs/adr/007-ffmpeg-h264-encoder-and-licensing.md)). Document version + `configure` line in NOTICE. Bundle at package time; do not commit the binary. |
| Cloud/storage | Google **Drive REST v3** via `fetch` + GIS | No heavyweight Google SDK. Behind the `StorageProvider` interface. **Implemented in the web/TS code only** — the Rust app does not call Drive (see §5). |

Adding a new runtime dependency requires justification against the *lightweight* invariant.
Prefer the platform API or a small utility over a framework. When in doubt, don't add it.

> **SUPERSEDED by [ADR-006](docs/adr/006-phase-2-tauri-shell-with-swappable-core.md) (2026-08-06):
> Phase 2 uses a Tauri v2 shell over a shell-agnostic core.** Tauri wins on installer +
> auto-update (ADR-005's open action items) and on the recorder chrome ADR-001 itself called
> cheap to lose. The reversal is affordable *because* ADR-001 was right that the hard parts —
> capture, audio, the bubble — are native regardless of shell. The escape hatch back to a
> pure-Rust shell is structural, not aspirational: see `read_this_idiot.md`.
> The historical rationale follows.
>
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
| [001](docs/adr/001-phase-2-pure-rust-desktop.md) | Phase 2 is pure Rust, not Tauri — **superseded by 006 (shell question only)**; the native-capture reasoning still stands | §2, §3 |
| [002](docs/adr/002-desktop-publish-loopback-origin.md) | Publish page served from a loopback HTTP origin | §5 |
| [003](docs/adr/003-phase-1-capture-pipeline.md) | Chunk-to-disk, container ladder, composite clock | §6 |
| [004](docs/adr/004-ffmpeg-sidecar-and-size-budgets.md) | FFmpeg as a separate process; size/CPU budgets | §1, §3 |
| [005](docs/adr/005-installed-app-not-portable.md) | Installed app + auto-update, not a portable exe | §1, §2 |
| [006](docs/adr/006-phase-2-tauri-shell-with-swappable-core.md) | Tauri v2 shell over a shell-agnostic core (supersedes 001) | §2, §3, §4 |
| [007](docs/adr/007-ffmpeg-h264-encoder-and-licensing.md) | `h264_mf` over `libx264`; the `configure` line (amends 004) | §3 |

Substituting a fixed choice above requires a **new ADR**, not an edit to an existing one.

---

## 4. Repository structure & boundaries

```
packages/                       # TypeScript / pnpm workspace (web side)
  core/      # shared TS: recorder state machine, domain types. PLATFORM-AGNOSTIC.
  storage/   # StorageProvider interface + providers (GoogleDriveProvider first).
  ui/        # shared React components. Browser-safe only. Used by the PWA and the publish
             #   page, and by the Phase 2 desktop shell's chrome (ADR-006 restored this;
             #   lands with M6 — the shell does not exist yet).
  publish/   # the "connect & upload" page (GIS token client + browser→Drive upload).
             #   loaded by BOTH the PWA and the Phase 2 desktop web view.
  web/       # Phase 1 PWA. Browser capture engine lives here.
apps/
  desktop/   # Phase 2 Cargo workspace (ADR-006): Tauri-free core crates + a replaceable shell.
    scripts/ #   check-shell-boundary.ps1 — the CI gate for the Tauri boundary.
    crates/
      vrecnu-types/           domain vocabulary. No platform dependencies.
      vrecnu-shell-api/       THE BOUNDARY: shell traits, events, AppError.
      vrecnu-capture/         screen capture (`windows-capture`/WGC) + the frame pacer.
      vrecnu-audio/           WASAPI loopback + mic (`wasapi`), and the silence filler.
      vrecnu-camera/          sole owner of the camera device (`nokhwa`).
      vrecnu-encode/          the FFmpeg sidecar (child process, never linked).
      vrecnu-namedpipe/       Windows named pipe that carries audio to ffmpeg.
      vrecnu-bubble-win32/    the floating webcam bubble, raw Win32.
      vrecnu-publish-server/  loopback HTTP server (`tiny_http`) for the publish page + recording.
      vrecnu-engine/          orchestration. Owns the threads, knows no shell.
      vrecnu-cli/             headless shell — the swappability proof. Keep it green.
      vrecnu-shell-tauri/     PLANNED (M6), not started — the ONLY crate allowed to name Tauri.
```

**Boundary rules (strict):**

- `packages/core` and `packages/storage` MUST NOT import browser-only or Node-only APIs.
  Keep them pure TS + web-standard APIs (`fetch`, Web Crypto) so they run in the browser
  (PWA and desktop web view alike).
- Platform-specific capture code (`getDisplayMedia` in web; Windows Graphics Capture/WASAPI
  in the Rust desktop crates) lives in its own app, **never** in shared packages.
- **The web/UI code talks to storage only through the `StorageProvider` interface.** No
  direct `drive.googleapis.com` calls in UI or recording code — Drive is one provider
  implementation, not a hardcoded dependency.
- **The Rust desktop crates do NOT implement or call Drive at all.** They record, edit,
  and expose the finished local file to the publish web view (§5). All OAuth and upload
  happen in `packages/publish` running in that web view. Do not add a Drive/OAuth/HTTP-to-
  Google dependency to any Rust crate.
- **Only `vrecnu-shell-tauri` may name `tauri`, `tao`, `wry` or `webview2-com`** (ADR-006),
  enforced by `apps/desktop/scripts/check-shell-boundary.ps1` in CI. Never put an
  `if (window.__TAURI__)` branch in `packages/publish` — the share link returns to the engine
  over `POST /api/complete` on the loopback origin, not Tauri IPC.

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
  - Web/PWA **and** the Phase 2 desktop publish web view: use the **Google Identity
    Services (GIS) token client** — the token model, a public client with no secret, no
    refresh token and ~1-hour access tokens. This is deliberately **not** authorization-code +
    PKCE: Google requires a client secret at the token endpoint for "Web application"
    clients, so secretless PKCE is not available to a static page. There is **one** OAuth
    implementation (`packages/publish`); the desktop reuses it by loading that page in a web
    view. The old native **loopback-redirect OAuth** flow is **not** used — do not add it.
  - The token getter (`getAccessToken()`, §6) **caches the current token until shortly before it
    expires**, then re-requests one via GIS. The **first** token request must originate from a
    user gesture (a click) — otherwise popup blockers suppress Google's consent window.
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
  - Gate the recording with a **single-session capability token** in the URL — one capability,
    scoped to one recording and one publish session, invalidated on completion, cancel or
    timeout. (Not per-request "one-time": the endpoint is necessarily fetched many times —
    Range windows, the `<video>` preview. See ADR-006's clarification.)
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
- **Container is chosen by feature detection, first match wins — and the ladder is
  WebM-first until Drive preview seek is verified.** [ADR-003](docs/adr/003-phase-1-capture-pipeline.md)
  §3.3 *prefers* MP4 but **forbids shipping the MP4-first ordering** until Chromium's
  fragmented-MP4 output is verified to play *and seek* in Google Drive's preview player. That
  verification has not happened, so the shipped ladder (`packages/core/src/types.ts`) is:
  1. `video/webm;codecs=vp9,opus`
  2. `video/webm;codecs=vp8,opus`
  3. `video/webm`
  4. `video/mp4;codecs=avc1.42E01E,mp4a.40.2`

  Gate each with `MediaRecorder.isTypeSupported()`. Only after the Drive check passes — and its
  result is recorded in ADR-003 (action item 2) — may MP4 move to the top. Do not reorder on
  assumption.
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
- **`GoogleDriveProvider` must take a `getAccessToken(): Promise<string>` callback, not a token
  string.** GIS access tokens last ~1 hour with no refresh token; a large recording on a slow
  uplink can outlive one. The resumable session URI survives re-auth, so the upload loop must
  be able to obtain a fresh token mid-upload. The getter itself caches the token until shortly
  before expiry and re-requests via GIS after that (§5).
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
- **Never commit secrets.** No OAuth secrets, API keys, tokens, or `.env` files. GIS access
  tokens are short-lived and session-scoped in the web page (held in memory, never written to
  the repo or a bundle); the Rust app never handles an OAuth token at all (§4).
- Add and respect `.gitignore` for `node_modules/`, `dist/`, `target/`, `.env*`,
  build output, and recordings.

---

## 8. Build, run & verify

The repo is scaffolded and `.github/workflows/ci.yml` runs the commands below on every PR.
Run the same ones locally before declaring work done.

**TypeScript workspace** — repo root (pnpm ONLY):

```bash
pnpm install               # bootstrap (CI uses `pnpm install --frozen-lockfile`)
pnpm dev                   # run the web app (Phase 1)
pnpm lint
pnpm typecheck
pnpm test
pnpm build                 # builds the PWA (packages/web) AND the standalone publish page (packages/publish)
pnpm --filter web budgets  # bundle budgets from §1 — run after `pnpm build`
```

**Rust desktop workspace** — `apps/desktop`, run in **PowerShell (or cmd), not Git Bash**
(Cargo commands, NOT `tauri` commands):

```powershell
cd apps/desktop
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
./scripts/check-shell-boundary.ps1   # ADR-006 gate: dependency-graph scan + link-time proof that
                                     #   vrecnu-cli builds with no Tauri. Runs in CI; do not delete it.
cargo build --workspace
cargo run -p vrecnu-cli -- help      # the headless shell; there is no Tauri shell yet (M6)
```

Desktop build traps (details in `apps/desktop/README.md`):

- **Build the publish page first.** `vrecnu-publish-server/build.rs` embeds
  `packages/publish/dist` into the binary at compile time, so run `pnpm build` (or just
  `pnpm --filter @vrecnu/publish build`) **before** `cargo build`/`cargo run` for anything that
  will publish. A missing `dist/` is *not* a build error — the workspace builds fine without
  Node — but the server then returns `PublishError::AssetsMissing` at runtime. Rebuild the
  page after changing `packages/publish`.
- **Git Bash shadows the linker.** Its GNU `link` hides MSVC's `link.exe` and fails with a
  misleading `link: extra operand`. Use PowerShell or cmd.
- **The Windows SDK is required** — even `cargo check` links proc-macro crates and build
  scripts. `LNK1181: cannot open input file 'kernel32.lib'` means the C++ desktop workload
  (with the SDK) is not installed.
- **`memory allocation of N bytes failed`** right after a Visual Studio/SDK install is
  parallelism, not code: cap it with `cargo test --workspace -j 2` until the installer settles.

**Verification is required before declaring work done:**

- **Web/TS:** `pnpm typecheck`, `pnpm lint` and `pnpm test` must pass (and `pnpm build` +
  `pnpm --filter web budgets` if you touched bundle size). No new type errors, no
  `// @ts-ignore` without a reason comment.
- **Rust (desktop):** `cargo fmt --all -- --check`,
  `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --workspace` and
  `./scripts/check-shell-boundary.ps1` must pass.
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
- ❌ Electron. ❌ Naming `tauri`/`tao`/`wry`/`webview2-com` in any crate except
  `vrecnu-shell-tauri`, and ❌ deleting `vrecnu-cli` — it is the link-time proof that the
  shell is still swappable (ADR-006). Formerly this line banned Tauri outright (ADR-001).
  Outside `vrecnu-shell-tauri`, a web view (or the system browser) exists **only** for the
  publish step (§5).
- ❌ Implementing Google Drive / OAuth / upload inside any Rust desktop crate — it lives in
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
  loopback-only, single-session-token, same-origin endpoint that is torn down after publish (§5).
- ❌ Reintroducing the native loopback-**redirect OAuth** flow (the GIS token client in the
  web page is the only OAuth path).
- ❌ Committing secrets, tokens, or `.env`.
- ❌ Client secrets in web/PWA bundles (the GIS token client is a public client — no secret).
- ❌ Non-resumable uploads.
- ❌ FFmpeg-heavy editing in the Phase 1 web app.
- ❌ Platform-specific imports in `packages/core` / `packages/storage`.
- ❌ Direct Drive API calls outside a `StorageProvider`.
- ❌ Adding dependencies that fight the lightweight invariant without justification.
- ❌ Any content telemetry.
- ❌ Marking work "done" without typecheck + lint + test (TS) / clippy + fmt + test + the
  shell-boundary script (Rust) + a real functional check.

When a task seems to require breaking one of these, **stop and ask the maintainer** rather
than working around it.
