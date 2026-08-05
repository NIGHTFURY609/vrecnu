# ADR-001: Phase 2 is a pure-Rust desktop app, not Tauri

**Status:** Accepted — **rationale amended by [ADR-005](005-installed-app-not-portable.md)**
**Date:** August 2026
**Deciders:** Maintainer (Jeswin Christie)

> **Read this first (2026-08-03).** The *decision* below still holds: Phase 2 is a pure-Rust
> native GUI and Tauri is still rejected. But the **reasoning has changed.** This ADR argues
> from *portability* — avoiding a WebView2 runtime dependency so the exe could run off a USB
> stick. [ADR-005](005-installed-app-not-portable.md) dropped the portability invariant, so
> that argument no longer applies.
>
> The decision was re-examined rather than grandfathered, and survives on independent
> grounds: **runtime footprint during capture** (~30–50 MB RSS native vs ~80–150 MB with a
> WebView). A recorder competes for CPU and RAM with the app being recorded, and a heavy one
> causes dropped frames. Note this *inverts* the size argument below — with an installer,
> Tauri actually produces a **smaller download** (~5 MB vs ~12–15 MB); it loses on resident
> cost, not on bytes shipped. Treat §"Options Considered" here as historical.

> **Note:** this ADR is a backfill. The decision was originally recorded as a blockquote in
> `AGENTS.md` §3 and in `ARCHITECTURE.md` §2a. It is restated here so the ADR numbering has
> a real origin and later ADRs have something to reference. The wording is normalized; the
> decision is unchanged.

## Context

vrecnu has four invariants: **lightweight, portable/no-install, bring-your-own-storage,
privacy.** Phase 2 delivers a portable Windows `.exe` that runs from a USB stick with no
installer and no admin rights.

The original plan was a **Tauri v2** shell wrapping the Phase 1 React frontend, with an
estimated ~70% frontend reuse between web and desktop. Tauri's UI is rendered by the
system WebView — on Windows, **WebView2**. That is a runtime dependency the exe does not
control: it is present on most current Windows installs but not guaranteed, and its absence
turns "download and run" into "download, fail, install a Microsoft runtime, retry."

The forces in tension:

| Force | Pulls toward |
|---|---|
| Portable, self-contained, runs anywhere | Native GUI with its own renderer |
| Contributor accessibility, code reuse | Tauri + the existing React frontend |
| Small binary, low idle cost | Native GUI |
| Fast time-to-Phase-2 | Tauri |

Critically, the *hard* parts of Phase 2 are native regardless of shell: `scap` screen
capture, WASAPI/`cpal` audio, and a transparent always-on-top webcam bubble window. Tauri
does not help with any of them.

## Decision

Phase 2 ships as a **pure-Rust desktop application** with a native Rust GUI (`egui` by
default; `Slint` or `iced` permitted). **Tauri is not used. Electron is forbidden.**

A WebView (`wry`) or the system browser is permitted **only** for the Google Drive publish
step — see [ADR-002](002-desktop-publish-loopback-origin.md).

## Options Considered

### Option A: Tauri v2 shell + shared React frontend

| Dimension | Assessment |
|---|---|
| Complexity | Low — reuses Phase 1 UI |
| Portability | **Compromised** — WebView2 runtime dependency |
| Binary size | Small exe, but relies on a large external runtime |
| Contributor familiarity | High (React/TS) |

**Pros:** fastest path; large shared frontend; Tailwind/shadcn available; Cap uses this shape and it works.
**Cons:** WebView2 dependency undermines the portability invariant; UI rendering behavior varies with the user's WebView2 version; two runtimes (Rust + JS) in one process.

### Option B: Pure-Rust native GUI (chosen)

| Dimension | Assessment |
|---|---|
| Complexity | Medium — recorder UI rebuilt natively |
| Portability | **Strong** — self-contained renderer, no external runtime |
| Binary size | Larger exe, no external runtime |
| Contributor familiarity | Lower — smaller Rust GUI ecosystem |

**Pros:** genuinely self-contained exe; no WebView2; native capture/audio/bubble code was going to be Rust anyway; one runtime.
**Cons:** loses the shared React frontend; no Tailwind/shadcn/React component pool; must build the recorder UI by hand; narrower contributor pool for desktop work.

### Option C: Dioxus (one Rust UI rendering to web + desktop)

**Pros:** restores a single shared frontend, in Rust.
**Cons:** reintroduces a WebView on desktop — i.e. it gives back reuse at the exact cost this decision was made to avoid. Held as the fallback if shared-frontend pressure ever outweighs the WebView-free exe.

## Trade-off Analysis

The decision trades **developer convenience for user-facing portability**. That is the
right direction for this project specifically, because "runs off a USB stick on any PC" is
a stated product invariant and a differentiator, whereas frontend reuse is an internal
efficiency concern.

The reuse loss is smaller than it first appears. The product's genuinely
hard-to-get-right code is the OAuth + resumable Drive upload path — and that stays in
TypeScript, written once, reused by the desktop through the publish web view (ADR-002).
What is *not* reused is the recorder chrome: buttons, source picker, preview. That is
rebuildable and deliberately kept simple until Phase 3.

## Consequences

**Easier:**

- The portable exe has no WebView2 runtime dependency for its own UI.
- One runtime, one language, in the desktop process.
- Native bubble/capture/audio work is unobstructed by a shell abstraction.

**Harder:**

- `packages/ui` now serves the **web app only**. The "~70% shared frontend" claim is void.
- The recorder UI must be built natively; budget real time for it.
- No Tailwind/shadcn on desktop; visual polish costs more effort.
- `egui` is immediate-mode and repaints continuously by default — see
  [ADR-004](004-ffmpeg-sidecar-and-size-budgets.md) for the CPU budget this must respect.

**To revisit:**

- If a single shared frontend ever becomes more valuable than a WebView-free exe,
  reconsider **Dioxus** (Option C) — and accept the WebView back on desktop.

## Action Items

1. [x] Record the decision in `AGENTS.md` §3 and `ARCHITECTURE.md` §2a.
2. [ ] Scaffold `apps/desktop` as a plain Cargo crate — **no `src-tauri/`**.
3. [ ] Prototype the transparent always-on-top bubble window with `winit` before
       committing to `egui` vs `Slint`.
