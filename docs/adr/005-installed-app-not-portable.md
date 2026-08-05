# ADR-005: Phase 2 ships as an installed app, not a portable exe

**Status:** Accepted
**Date:** 2026-08-03
**Deciders:** Maintainer (Jeswin Christie)
**Amends:** [ADR-001](001-phase-2-pure-rust-desktop.md) (rationale, not decision)

## Context

"**Portable / no-install**" has been one of vrecnu's four invariants since the project
started, and it has been load-bearing: [ADR-001](001-phase-2-pure-rust-desktop.md) chose a
pure-Rust GUI over Tauri *specifically* because Tauri's WebView2 runtime dependency
threatened the self-contained `.exe`. Remove the portability requirement and ADR-001's stated
justification disappears with it — which is why this ADR has to address both.

Reassessing portability honestly, it costs more than it gives:

- **No auto-update.** This is the decisive problem. vrecnu is coupled to Google's OAuth
  behavior and the Drive REST API — third-party surfaces that change on Google's schedule,
  not ours. A portable exe sitting in someone's `Downloads` folder cannot be fixed when that
  happens; it is simply broken forever, and the user's conclusion is "vrecnu is broken," not
  "vrecnu is out of date."
- **SmartScreen never stops warning.** An unsigned or reputation-less portable binary
  re-triggers the Windows warning on essentially every download. An installed, signed app
  accumulates reputation once.
- **Real-world portable-app behavior is bad.** Users lose the exe, run stale copies
  indefinitely, and have no clean uninstall.
- **The USB-stick scenario is narrow** compared to the cost of carrying all of the above.

Meanwhile, **the "no install" promise does not actually depend on the desktop app.** Phase 1
is a PWA that runs in any Chromium browser with literally nothing to install. The no-install
answer already exists and always will. Phase 2 was duplicating a promise that Phase 1 keeps
better.

There is one genuine loss: "no install" was a *differentiator* against Cap and friends. But
the real differentiator was always **bring-your-own-storage**, and that is untouched here.

## Decision

**Phase 2 ships as an installed Windows application**, not a portable executable.

- **Per-user install by default** — no administrator rights required.
- **Auto-update** is a first-class feature, not a Phase 3 nice-to-have. It is the main reason
  this decision was made.
- **Code-signed** releases.
- Clean uninstall that removes the app and leaves the user's recordings alone.

**Invariant 2 is reworded** in `AGENTS.md` §1, from:

> **Portable / no-install.** Web app runs in any browser; desktop ships as a portable `.exe`
> (no installer, no admin rights).

to:

> **A no-install path always exists.** The web app (Phase 1) runs in any browser with nothing
> to install, and that is permanent. The desktop app installs cleanly — per-user, no admin
> rights, auto-updating, clean uninstall.

**[ADR-001](001-phase-2-pure-rust-desktop.md)'s decision stands — its rationale is replaced.**
The desktop app remains a **pure-Rust native GUI**, and Tauri remains rejected, but *not*
because of portability any more. The new reason is **runtime footprint**:

| | Native Rust GUI | Tauri / WebView2 |
|---|---|---|
| Installer size | ~12–15 MB | ~5 MB (WebView2 already present) |
| RSS while running | ~30–50 MB | ~80–150 MB |

Tauri wins on download size and loses badly on runtime cost. **For a screen recorder, runtime
cost is the one that matters**: the app competes for CPU and RAM with the application being
recorded, and a heavy recorder directly causes dropped frames — the exact failure
[ADR-003](003-phase-1-capture-pipeline.md) is built to prevent. A one-time 10 MB download is
not worth a permanent 100 MB resident cost during capture.

Confirmed alongside this decision: **the stripped FFmpeg sidecar of
[ADR-004](004-ffmpeg-sidecar-and-size-budgets.md) stays.** Media Foundation, `openh264`,
`rusty_h264`, and pure-Rust muxers were evaluated as ways to reach ~0 MB; ~25 MB is
acceptable, and FFmpeg's maturity on the path where bugs destroy recordings is worth more
than the saving. The separate-process requirement and the size budget are unchanged.

## Options Considered

### Option A: Portable exe only (status quo)

| Dimension | Assessment |
|---|---|
| Distribution | Single file, no install |
| Update story | **None** |
| Trust/SmartScreen | Poor — warning recurs |
| Fit with Google-API coupling | **Bad** |

**Pros:** USB-stick use; no admin; genuinely nothing to install; a marketing line.
**Cons:** cannot be updated when Google changes something, which is a matter of *when*; permanent SmartScreen friction; stale versions in the wild forever; no clean uninstall.

### Option B: Installed app only (chosen)

| Dimension | Assessment |
|---|---|
| Distribution | Installer, per-user |
| Update story | **Auto-update** |
| Trust/SmartScreen | Good — reputation accrues |
| Fit with Google-API coupling | **Good** |

**Pros:** fixes the update problem outright; better trust; clean uninstall; stable install directory; standard Windows integration.
**Cons:** loses the USB-stick scenario and the "no install" line for desktop; adds installer tooling, signing, and update-server infrastructure to build and maintain.

### Option C: Ship both

**Pros:** maximum reach; nobody is excluded.
**Cons:** two distribution channels and two support stories, and the portable build *still* can't auto-update — so the worst failure mode survives, now with the added confusion of two builds behaving differently. Rejected: it keeps the problem while doubling the work.

## Trade-off Analysis

The core trade is **a marketing line for a maintenance capability**, and for this project
that is clearly the right direction. vrecnu's correctness depends on a third party we do not
control. An app that cannot be updated is an app that is guaranteed to break — the only open
question is when. No amount of portability compensates for shipping software you can never
fix.

The decision is also less costly than it appears, because **Phase 1 absorbs the loss
entirely**. The user who wants zero installation opens the web app; that path is not
degraded by this change at all. What is actually being given up is narrower than "no-install"
— it is specifically "run the *desktop* app from removable media on a machine you don't
control." That is a small population, and it is the population least likely to need the
desktop-only features (floating bubble, native system audio) in the first place.

Keeping ADR-001's decision while discarding its rationale deserves scrutiny, since a decision
whose justification evaporated is exactly the kind that should be re-examined rather than
grandfathered. It was re-examined, and it survives on independent grounds: runtime footprint
during capture is a first-order concern for a recorder, and it favors native Rust more
strongly than portability ever did. The frontend-reuse argument for Tauri is real but is an
internal efficiency concern, and it loses to a user-facing capture-quality concern.

The honest cost that remains is **infrastructure**: auto-update requires an update feed, a
signing key with a real custody story, and a release process that can't be done by hand.
That is new operational work for a project whose architecture was otherwise proudly
backend-free. Note that this does **not** breach the no-server invariant — an update feed
serves static release metadata and never touches recordings.

## Consequences

**Easier:**

- Broken Google integrations can actually be fixed for existing users.
- SmartScreen reputation accrues to one signed installer.
- Stable install directory — bundled publish assets ([ADR-002](002-desktop-publish-loopback-origin.md)) have a predictable home.
- Clean uninstall; standard Windows integration.
- Runtime footprint stays low, protecting the frame-drop budget.

**Harder:**

- Installer tooling, code signing, and key custody are now required.
- An update feed must be built, hosted, and kept static.
- The "no install" pitch applies to the web app only — update README and any marketing copy.
- Users on locked-down machines lose the desktop app (the PWA still works).

**To revisit:**

- If auto-update infrastructure proves too heavy to maintain, revisit **Option C** — but only
  with a plan for how portable users learn they are stale.
- If the desktop app ever needs to run where installation is impossible, that is a new ADR.
- ADR-001's Dioxus fallback (Option C there) remains the escape hatch if shared-frontend
  pressure ever outweighs runtime footprint — the WebView objection is now about RSS, not
  portability.

## Action Items

1. [ ] Reword invariant 2 in `AGENTS.md` §1.
2. [ ] Update `AGENTS.md` §2 and `ARCHITECTURE.md` §2/§3 phase descriptions: installed app,
       not portable exe.
3. [ ] Mark `ADR-001` as rationale-amended by this ADR.
4. [ ] Choose installer tooling and record it (per-user, no admin).
5. [ ] Establish code-signing key custody before first release.
6. [ ] Design the update feed as **static** hosting — it must not become a backend.
7. [ ] Update `README.md` so "no install" refers to the web app.
