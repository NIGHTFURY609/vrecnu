# ADR-004: FFmpeg ships as a separate process; numeric size and CPU budgets

**Status:** Accepted — **reaffirmed 2026-08-03**
**Date:** 2026-08-02
**Deciders:** Maintainer (Jeswin Christie)
**Relates to:** [ADR-001](001-phase-2-pure-rust-desktop.md), [ADR-005](005-installed-app-not-portable.md)

> **Reaffirmed 2026-08-03.** Whether to drop FFmpeg entirely was reconsidered. Windows Media
> Foundation (encode/mux at ~0 MB via the `windows` crate), raw-frame pixel adjustments
> pre-encode, `ebur128` for loudness, and pure-Rust MP4 muxers (`mp4e`, `muxide`) together
> could have replaced it for roughly **zero** bundled bytes and removed the licensing concern
> outright. **Rejected:** 25 MB is acceptable, and FFmpeg's maturity is worth more on a path
> where bugs destroy recordings. Media Foundation is also COM-heavy and Windows-only, and the
> pure-Rust encoders (`less-avc` is lossless-only; `rusty_h264` is very new; `openh264` is
> software-only) are not ready to be load-bearing for realtime 1080p30. The separate-process
> requirement and every budget below are unchanged. Wording updated for the installed-app
> model (ADR-005) — the numbers did not move.

## Context

Two problems, joined because they are the same problem wearing different hats: **FFmpeg is
the largest single threat to the "lightweight" invariant, and "lightweight" is currently
unenforceable.**

### Licensing

vrecnu is **MIT licensed** (`LICENSE`, © 2026 Jeswin Christie). `AGENTS.md` §3 currently
specifies encoding/editing as *"FFmpeg as a bundled sidecar (or `ffmpeg-next` bindings)"* —
presenting the two as interchangeable. They are not.

FFmpeg is LGPL-2.1+ at minimum, and GPL if built with GPL-only components (`x264`, several
filters). Invoking `ffmpeg.exe` as a **separate process** is aggregation: the two programs
communicate at arm's length, and our MIT terms are unaffected. **Linking** FFmpeg into our
binary via `ffmpeg-next` is different: under LGPL it triggers relinking/notice obligations
that a statically-linked single-file portable exe is poorly placed to satisfy, and if any
GPL component is present it is a straightforward license conflict with distributing an MIT
binary. Offering the two as equal options invites a contributor to pick the one that creates
a legal problem, in a public repo, silently.

### Size

A stock Windows `ffmpeg.exe` is roughly **70–90 MB**. `AGENTS.md` §1 declares
*"Lightweight. Small footprint, fast, minimal dependencies. Bloat is a bug."* and
`ARCHITECTURE.md` describes the Phase 2 deliverable as "a small folder (exe + ffmpeg.exe)."
A ~15 MB Rust exe shipped beside an 85 MB FFmpeg is not a small folder — FFmpeg would be
roughly six times the app. The "USB-stick portable" pitch does not survive that.

### Enforceability

"Bloat is a bug" cannot fail a review, because nothing states what bloat *is*. There is no
number anywhere in `AGENTS.md` or `ARCHITECTURE.md` for binary size, bundle size, memory, or
CPU. Invariants without thresholds get eroded one reasonable-looking dependency at a time.

Compounding this on the desktop side: `egui`/`eframe` (ADR-001) defaults to **continuous
repaint**, redrawing every frame whether or not anything changed. That burns CPU precisely
while the user is recording — the one moment CPU headroom matters most, because it directly
causes dropped frames.

## Decision

### 4.1 — FFmpeg is a separate process. Always.

The Phase 2 desktop app invokes a bundled `ffmpeg.exe` as a **child process**. Linking
FFmpeg into the vrecnu binary — via `ffmpeg-next`, `ffmpeg-sys`, or any other bindings crate
— is **forbidden**. The "(or `ffmpeg-next` bindings)" alternative is struck from
`AGENTS.md` §3.

Additionally:

- Prefer an **LGPL** build; do not enable GPL-only components unless a specific feature
  requires one, and if that ever happens, reopen this ADR first.
- Ship `NOTICE` with FFmpeg's license text, version, and exact `configure` line.
- Do **not** commit the binary — fetch and verify it at package time.

### 4.2 — Ship a stripped FFmpeg build

Build FFmpeg with only what vrecnu uses, rather than shipping the everything-build. Enable
only: H.264/AAC encode + decode, VP9/Opus decode, MP4 and WebM mux/demux, and the filters
actually used (`eq`, `loudnorm`, `volume`, `highpass`, `atrim`/`trim`). Disable
`ffplay`, `ffprobe` if unused, docs, network protocols, and all other codecs.

**Target: ≤ 25 MB.** A stripped build in the ~15–25 MB range is achievable and keeps the
sidecar smaller than the "small folder" claim can bear.

### 4.3 — Numeric budgets, enforced in review

These become part of `AGENTS.md`. Exceeding one is a blocking review finding, not a
discussion:

| Budget | Limit |
|---|---|
| PWA initial JS (gzipped, first load) | **≤ 150 KB** |
| PWA total transferred, first load | **≤ 400 KB** |
| Desktop exe (release, stripped, no FFmpeg) | **≤ 15 MB** |
| Bundled FFmpeg sidecar | **≤ 25 MB** |
| Total installed footprint (exe + FFmpeg) | **≤ 45 MB** |
| Desktop idle CPU (not recording) | **≤ 2%** |
| Desktop idle RSS (not recording) | **≤ 150 MB** |
| Recording CPU overhead, 1080p30 | **≤ 10%** of one core above baseline capture cost |
| Dropped frames, 1080p30, 5-minute recording | **0** |

### 4.4 — `egui` runs in reactive repaint mode

The desktop UI must not repaint continuously. Use `eframe`'s reactive mode and explicit
`request_repaint_after()` for timers (elapsed-time display, level meters). The floating
bubble window repaints on camera frames only. This is what makes the 2% idle budget
attainable rather than aspirational.

## Options Considered

### FFmpeg integration

| Option | License risk | Binary size | Ergonomics | Robustness |
|---|---|---|---|---|
| **Sidecar process (chosen)** | **None** — aggregation | +25 MB on disk, 0 in exe | CLI args, parse stderr | A crash kills the child, not the app |
| `ffmpeg-next` static link | **High** — LGPL relinking / GPL conflict with MIT | Large exe | Typed API, no parsing | An FFmpeg crash takes the app down |
| `ffmpeg-next` dynamic link | Medium — LGPL satisfiable | Still ships DLLs | Typed API | Version-matching pain |
| No FFmpeg (WebCodecs-style native encode) | None | Smallest | Large amount of custom code | Rewrites what FFmpeg already does |

**Pros of chosen:** eliminates the licensing question entirely; keeps the exe small; isolates
crashes; the sidecar can be swapped or updated without rebuilding vrecnu.
**Cons:** process-spawn overhead per operation; progress and errors must be scraped from
FFmpeg's stderr, which is brittle across versions (pin the version); an extra file in the
"portable folder," so it is not literally one file.

### Budgets

| Option | Assessment |
|---|---|
| No numbers (status quo) | Invariant is unenforceable; erosion is invisible until it's bad |
| **Explicit budgets (chosen)** | Enforceable; some numbers will be wrong at first |
| Automated CI size gate | Better still — but the repo is pre-scaffold; do it when CI exists |

## Trade-off Analysis

On FFmpeg, the ergonomic loss from process-spawning is real but small, and it buys away a
legal risk that a public MIT repo genuinely cannot afford to carry casually. Crash isolation
is a meaningful secondary win for a recorder: FFmpeg dying during export should lose an
export, not the recording.

The stripped-build requirement adds real packaging work — maintaining a `configure` line and
a reproducible build step is more effort than downloading a release binary. It is accepted
because a 90 MB dependency contradicts a headline product claim, and shipping an
80-plus-percent-unused binary is exactly the "bloat is a bug" case.

On budgets, the honest risk is that **specific numbers chosen before any code exists will be
wrong.** That is fine and expected. A wrong number that forces a conversation is more useful
than no number that allows silent drift — and revising a budget with measurements in hand is
a legitimate, cheap ADR amendment. The numbers are calibrated to be *achievable but not
comfortable*.

## Consequences

**Easier:**

- The licensing story is unambiguous and defensible for a public MIT release.
- "Is this too heavy?" has an answer a reviewer can check.
- FFmpeg failures are contained.
- The portable folder stays plausibly portable.

**Harder:**

- Packaging must fetch, verify, and possibly build a custom FFmpeg — more release machinery.
- FFmpeg progress/error handling is stderr parsing; pin the version and test against it.
- Budgets must be measured, which means building the measurement early.
- Reactive repaint is slightly more effort than immediate-mode's default loop.

**To revisit:**

- Add a CI size-gate the moment CI exists — budgets checked by hand will drift.
- Re-measure and revise the numbers once Phase 1 and Phase 2 have real builds; treat the
  first revision as expected, not as failure.
- If a needed feature turns out to require a GPL-only FFmpeg component, reopen **4.1**
  before enabling it.

## Action Items

1. [x] Strike "(or `ffmpeg-next` bindings)" from `AGENTS.md` §3; add the linking ban to §10.
2. [x] Add the budget table to `AGENTS.md` §1.
3. [ ] Write the FFmpeg `configure` line and record it in `NOTICE` alongside the license text.
4. [ ] Add a packaging step that fetches/builds and checksum-verifies FFmpeg; never commit it.
5. [ ] Configure `eframe` for reactive repaint when `apps/desktop` is scaffolded.
6. [ ] Add size/CPU measurement to the build once CI exists, and gate on the table above.
