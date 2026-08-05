# ADR-003: Phase 1 capture pipeline — chunk-to-disk, container choice, composite clock

**Status:** Accepted (§3.1 amended 2026-08-03 — see [Amendment](#amendment-2026-08-03--opfs-replaces-the-save-picker))
**Date:** 2026-08-02
**Deciders:** Maintainer (Jeswin Christie)

## Context

`ARCHITECTURE.md` §3 and `AGENTS.md` §6 specify *what* the Phase 1 web recorder produces
(WebM via `MediaRecorder`, canvas-composited bubble, capped draw rate) but leave four
mechanical questions unanswered or answered wrongly. All four affect **reliability**, which
is a stated invariant, and three of them are cheap to fix now and expensive to fix later.

**1. Where do recorded bytes live?** Nothing says. The default `MediaRecorder` pattern
accumulates `ondataavailable` chunks in a JS array and concatenates to a `Blob` on stop. A
30-minute 1080p recording is several hundred MB held in tab memory. A tab crash, an OOM, or
an accidental navigation loses the entire recording with no recovery — after the user has
already spent 30 minutes making it. This is the worst failure mode the product has.

**2. The output container has no duration.** `MediaRecorder` WebM output omits the
`Info/Duration` element and the `Cues` index, because the browser does not know the total
length until recording stops and those elements live near the start of the file. Players
report duration `Infinity`, the seek bar does not work, and scrubbing is broken. Since the
entire product is "send someone a link," shipping links to unscrubbable video defeats the
purpose.

**3. WebM was chosen as the default before MP4 recording was viable.** Chromium now supports
recording directly to MP4/H.264 (`video/mp4;codecs=avc1.42E01E,mp4a.40.2`). Our stated target
is Chromium on Windows. MP4 has broader downstream compatibility and better behavior in
third-party players, and — being written with proper metadata — sidesteps problem 2 entirely.

**4. "Cap the canvas draw to source FPS" has no mechanism.** The naive implementation is a
`requestAnimationFrame` loop, which is tied to *display* refresh, not to the capture stream.
On a 144 Hz monitor it burns CPU redrawing unchanged frames; under load it drifts out of step
with the source and produces the desync/frame-drop failure `ARCHITECTURE.md` §7 already flags
as the riskiest part of Phase 1.

## Decision

Four decisions, adopted together as the Phase 1 capture pipeline.

### 3.1 — Stream chunks to disk, never accumulate in memory

> **Amended 2026-08-03.** The storage mechanism below was superseded by OPFS — see the
> [Amendment](#amendment-2026-08-03--opfs-replaces-the-save-picker) at the end of this
> document. The *principle* (never accumulate in memory) is unchanged.

`MediaRecorder` runs with a **`timeslice`** (start at 2000 ms; tune with measurement). Each
`ondataavailable` chunk is written straight out:

- ~~**Primary:** File System Access API — `showSaveFilePicker()` → `createWritable()` →
  `write()` per chunk.~~ *(superseded — see Amendment)*
- **Fallback:** IndexedDB, one record per chunk, assembled on stop.

On startup, if chunks from an unfinished session are present, offer **crash recovery**.

Upload reads from disk, not from an in-memory `Blob`.

### 3.2 — Repair container metadata before upload

Whatever container is produced, the file handed to the upload layer must have a valid
duration. For WebM output, patch the missing `Info/Duration` using a minimal
duration-patching utility (e.g. `fix-webm-duration` / `webm-fix-duration`, ~2 KB) with the
recorded wall-clock duration.

This is a metadata patch, **not** a transcode, and therefore does **not** violate the
"no FFmpeg-heavy editing in Phase 1" rule in `AGENTS.md` §2.

### 3.3 — Prefer MP4, fall back to WebM

Select the container by feature detection at record time, first match wins:

```
1. video/mp4;codecs=avc1.42E01E,mp4a.40.2     (MP4 / H.264 + AAC)
2. video/webm;codecs=vp9,opus
3. video/webm;codecs=vp8,opus
4. video/webm                                  (last resort)
```

`MediaRecorder.isTypeSupported()` gates each. **This supersedes the "default output is
WebM" rule in `AGENTS.md` §6.**

**Conditional on verification:** Chromium's MP4 recording output is fragmented MP4, which
some players handle poorly. Before this ordering ships, a recorded MP4 must be verified to
play *and seek* correctly in Google Drive's built-in preview player. If it does not, MP4
drops below WebM in the list and 3.2 carries the load. **Do not ship the MP4-first ordering
on assumption — measure it.**

### 3.4 — Drive the composite from `requestVideoFrameCallback`

The canvas composite loop is driven by `HTMLVideoElement.requestVideoFrameCallback()` on the
**screen** video element — not `requestAnimationFrame`. Each real source frame triggers
exactly one composite and one canvas draw. The webcam element is sampled at whatever state
it is in when the screen frame arrives.

This is the concrete mechanism behind the existing "cap draw rate to source FPS" rule.

## Options Considered

### Storage of recorded bytes

| Option | Memory | Crash-safe | Complexity |
|---|---|---|---|
| In-memory `Blob` (default) | **Bad** — grows unbounded | **No** | Trivial |
| IndexedDB chunks | Good | Yes | Low |
| **File System Access API (chosen)** | **Best** — flat | **Yes** | Low–medium |

**Pros of chosen:** flat memory regardless of length; recording survives a tab crash; the
user picks the destination up front so there is no "download" step; upload streams from disk.
**Cons:** requires a user gesture and a save dialog *before* recording starts, which adds one
click to the flow; needs the IndexedDB fallback path maintained alongside.

### Container

| Option | Compatibility | Duration metadata | Risk |
|---|---|---|---|
| WebM only (current rule) | Good on modern browsers | **Broken** without patch | Low |
| MP4 only | Best | Fine | Fails where unsupported |
| **MP4-first, WebM fallback (chosen)** | Best available per browser | Fine / patched | Fragmented-MP4 player quirks |

### Composite clock

| Option | CPU | Sync fidelity |
|---|---|---|
| `requestAnimationFrame` | Wasteful on high-refresh displays | Drifts from source under load |
| Fixed `setInterval` at target FPS | Predictable | Drifts; no relation to actual frames |
| **`requestVideoFrameCallback` (chosen)** | Proportional to real frames | Locked to source |

## Trade-off Analysis

The through-line is that all four decisions trade a small amount of implementation effort for
removal of a **silent** failure. Silent failures are especially bad here: a lost recording, a
broken seek bar, and dropped frames all surface *after* the user has finished recording, when
the cost of the failure is highest and the user has no way to recover.

The one decision with a genuine UX cost is **3.1**: the File System Access API needs a save
dialog before recording starts, which slightly dulls the "hit record instantly" feel. This is
accepted because the alternative is occasionally destroying half an hour of someone's work.
The dialog can be made once-per-session rather than once-per-recording.

**3.3** is the one decision taken with residual uncertainty, and it is deliberately written
as conditional. Preferring MP4 is right *if* Drive's player handles Chromium's fragmented
output; if it does not, the ordering flips and nothing else in this ADR changes.

## Consequences

**Easier:**

- Memory is flat and independent of recording length — long recordings stop being a risk
  category.
- Recordings survive crashes.
- Shared links behave correctly in other people's players (duration, scrubbing).
- Upload streams from disk, which composes cleanly with resumable upload and with a future
  upload-while-recording optimization.
- Frame pacing is correct by construction rather than by tuning.

**Harder:**

- Two storage paths (File System Access + IndexedDB fallback) to build and test.
- A save-location prompt before recording begins.
- Crash-recovery state to manage and garbage-collect.
- Codec selection is now branching logic, so **both** MP4 and WebM paths need test coverage.
- MP4-in-Drive behavior must be verified before the ordering ships.

**To revisit:**

- **Upload-while-recording.** Drive's resumable upload accepts chunks of unknown total size,
  so streaming to Drive *during* recording is possible and would make the link near-instant.
  Deliberately **not** Phase 1: it conflicts with post-record trim, and trim is the one
  editing feature Phase 1 has. Revisit in Phase 3.
- If `MediaRecorder` gains proper duration metadata, 3.2 can be dropped.

## Action Items

1. [ ] Recorder spike must prove chunk-to-disk **and** `requestVideoFrameCallback` pacing
       before any UI work — these are the load-bearing risks.
2. [ ] Verify a Chromium-recorded MP4 plays and seeks in Google Drive's preview player;
       record the result in this ADR before shipping the codec ordering.
3. [x] Update `AGENTS.md` §6 to replace "default output is WebM" with the detection ladder.
4. [ ] Add the duration patch to the pre-upload path, with a test asserting the output has a
       finite duration.
5. [ ] Implement crash recovery on startup, plus cleanup of abandoned chunk sets.

---

## Amendment (2026-08-03) — OPFS replaces the save-picker

**Amends §3.1 only.** Everything else in this ADR is unchanged.

### Why

Two things prompted the revision. First, §3.1's own "Trade-off Analysis" flagged a real UX
cost: `showSaveFilePicker()` requires a **save dialog before recording starts**, which dulls
the "hit record instantly" feel. Second, the question came up of whether writing to disk
during capture is too slow, and whether a **vrecnu-hosted relay server** should hold chunks
instead. Working through that surfaced a better local answer and settled the server question
on the numbers.

**Disk is nowhere near a bottleneck.** 1080p30 out of `MediaRecorder` is roughly 5 Mbps —
about **625 KB/s**; even 4K60 at high bitrate is ~5 MB/s. A slow SATA SSD sustains 500 MB/s.
Writes are async, absorbed by the OS page cache, and issued once per timeslice rather than
per frame. The H.264 encoder running alongside costs orders of magnitude more. For scale, an
OPFS sync access handle writes 100 MB in ~90 ms — it needs to absorb 0.6 MB/s.

**Memory is the option that actually fails, and not gracefully.** Recording 4K for ~20
minutes has been observed to push a tab past 3 GB, at which point Chrome's OOM killer takes
the process and the entire recording is lost. Chrome does sometimes spill large blobs to
disk, but that is an implementation detail we do not control and it does not survive a crash.
The decisive point is not throughput at all: **in RAM a crash loses everything; on disk the
file is already there.**

**A relay server is rejected**, and not only because it is unnecessary. It would route
recording bytes through vrecnu infrastructure, which is a direct violation of the privacy
invariant (`AGENTS.md` §7) and the one thing that distinguishes this project. It would also
impose storage and egress costs that grow with adoption, make the maintainer a data
controller with GDPR/DMCA/abuse exposure, and make the upload *slower* — user→server→Drive is
two sequential transfers on the user's uplink where browser→Drive is one. See `AGENTS.md`
§5/§7; this remains a hard architectural boundary.

### Amended decision

Recorded chunks are written to the **Origin Private File System**:

- `navigator.storage.getDirectory()` → `getFileHandle(name, {create:true})` →
  **`createSyncAccessHandle()` inside a dedicated Web Worker**, which is the only context
  where the sync handle is available and is the fastest binary path the platform offers
  (roughly an order of magnitude faster than IndexedDB for this workload).
- **No user prompt.** OPFS is origin-private storage; the save dialog disappears entirely,
  restoring the instant-record feel.
- Upload streams directly from the OPFS file.
- **Saving a local copy becomes an optional action at the end**, via `showSaveFilePicker()`,
  only when the user asks for it — which is where that dialog always belonged.
- Before a long recording, call **`navigator.storage.estimate()`** and warn if headroom is
  insufficient. Call **`navigator.storage.persist()`** so the data is not evicted mid-session.
- **Fallback** remains IndexedDB chunks. Support is broad enough (Chrome/Edge 86+,
  Firefox 111+, Safari 15.2+) that this tier should be rare.

### Consequences of the amendment

**Easier:** no pre-record prompt; faster writes than the previous plan; crash recovery is
unchanged and still works; one storage path covers essentially all target browsers.

**Harder:** OPFS is invisible to the user, so abandoned recordings must be garbage-collected
deliberately or they silently consume quota. Quota is finite and origin-scoped, so the
`estimate()` check is mandatory rather than optional. Sync access handles are worker-only, so
the recorder needs a worker boundary it previously did not.

**Unchanged:** everything in §3.2, §3.3, §3.4, and the "never accumulate in memory" principle.
