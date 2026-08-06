# ADR-007: The H.264 encoder — resolving ADR-004's LGPL contradiction

**Status:** Accepted
**Date:** 2026-08-06
**Deciders:** Maintainer (Jeswin Christie)
**Amends:** [ADR-004](004-ffmpeg-sidecar-and-size-budgets.md) §4.1 and §4.2

## Context

ADR-004 §4.2 requires the stripped FFmpeg build to enable "**H.264/AAC encode + decode**".
ADR-004 §4.1 separately requires: "Prefer an **LGPL** build; do not enable GPL-only components
unless a specific feature requires one, and if that ever happens, **reopen this ADR first**."

**These two requirements are in direct conflict, and nothing has caught it because ADR-004
action item 3 — "write the FFmpeg `configure` line" — is still unchecked.**

> `libx264` is GPL. FFmpeg refuses to build it without `--enable-gpl`. An LGPL FFmpeg build
> has **no H.264 encoder at all**.

The conflict would have surfaced at packaging time (M8), long after the whole capture pipeline
had been built around an ffmpeg command line that could not be produced under the stated
licence policy. ADR-004 §4.1 says to reopen before enabling a GPL component; this ADR does
that, and concludes we do not need to.

Shipping a GPL `ffmpeg.exe` beside an MIT application is *legal* — ADR-004 establishes that
separate-process invocation is aggregation. But it contradicts ADR-004's own stated preference
and pulls GPL text into `NOTICE` for a project whose licence posture is a deliberate feature.

## Decision

**Primary H.264 encoder: `h264_mf`** — FFmpeg's Media Foundation encoder wrapper.

- Built into FFmpeg on Windows; requires **no external library**, so it is LGPL-clean.
- Hardware-accelerated wherever the machine exposes an MF encoder (Intel QSV, NVIDIA, AMD),
  which is every GPU of the last decade. This directly serves the ≤10% CPU overhead and
  0-dropped-frames budgets in AGENTS.md §1.
- Size cost is approximately **zero** — no library to link, unlike `libx264`.
- The H.264 patent licence rides along with Windows rather than with our binary.

**Fallback: `libopenh264`** (BSD-2-Clause, LGPL-compatible, ~1 MB) for machines where no
Media Foundation H.264 encoder is present.

**Audio: FFmpeg's native `aac` encoder.** Built in, LGPL, no external library, and entirely
adequate at 128 kbps. `libfdk_aac` is **not** used — it is not redistributable.

### `configure` line

Recorded here to satisfy ADR-004 action item 3. Pin the FFmpeg version and reproduce this
verbatim in `NOTICE` alongside the licence text.

```
--disable-everything --disable-doc --disable-ffplay --disable-network --disable-autodetect
--enable-shared=no --enable-small --enable-lto --enable-version3
--enable-encoder=h264_mf,aac,libopenh264 --enable-libopenh264
--enable-decoder=h264,aac,vp9,opus,pcm_s16le,mjpeg
--enable-parser=h264,aac,vp9,opus
--enable-muxer=mp4,webm,ipod,null
--enable-demuxer=mov,matroska,webm_dash_manifest,wav,rawvideo,s16le
--enable-protocol=file,pipe
--enable-filter=eq,loudnorm,volume,highpass,atrim,trim,scale,format,setpts,asetpts,aresample,anull,null,copy,overlay
--enable-bsf=h264_mp4toannexb,extract_extradata
--enable-swscale --enable-swresample --disable-postproc
```

Notes on specific flags:

- `--enable-protocol=file,pipe` and no others. Audio reaches ffmpeg over a Windows named pipe
  (`\\.\pipe\…`), which `CreateFileW` serves through the `file` protocol — so the named-pipe
  design does **not** require re-enabling network protocols.
- `overlay` and `scale` extend ADR-004 §4.2's otherwise-closed filter list; `overlay` burns
  the webcam bubble into the recording. Recorded in [ADR-006](006-phase-2-tauri-shell-with-swappable-core.md) too.
- `--enable-version3` is required by `libopenh264`. It is LGPL **v3**, not GPL — it does not
  make the build GPL.

### `NOTICE` obligations

- FFmpeg's LGPL text, the pinned version, and the `configure` line above, verbatim.
- An honest note on `libopenh264`: **Cisco's royalty-free patent grant covers Cisco's own
  prebuilt binary, not an FFmpeg-internal build of the source.** Overstating that would be
  worse than saying nothing.

## Options Considered

| Option | Licence | Size | Speed | Verdict |
|---|---|---|---|---|
| `libx264` | **GPL** | ~1–2 MB | Excellent | Rejected — contradicts ADR-004 §4.1 |
| **`h264_mf`** | LGPL | ~0 MB | Hardware | **Chosen** |
| `libopenh264` | BSD / LGPLv3 | ~1 MB | Software only | Fallback |
| `libfdk_aac` | Non-redistributable | — | — | Rejected outright |
| Native `aac` | LGPL | ~0 MB | Fine at 128k | Chosen |

ADR-004 already evaluated and rejected Media Foundation as a *replacement for FFmpeg entirely*
("COM-heavy and Windows-only"). That rejection is not disturbed: this uses MF **through**
FFmpeg, as one encoder behind FFmpeg's own abstraction, so none of the COM surface reaches our
code and the muxing, filtering and trimming stay with FFmpeg.

## Consequences

**Easier:** the licence posture stays clean; GPU encoding comes free; the sidecar shrinks
rather than grows.

**Harder:** `h264_mf` quality at a given bitrate is below `libx264`, and its behaviour varies
by GPU vendor. Encoder selection must be probed at runtime with a fallback to `libopenh264`,
and the resulting encoder should be surfaced in diagnostics so a quality report can be
attributed.

**To revisit:** if `h264_mf` output quality proves unacceptable in M2 measurements, the next
option is a GPL build — and that requires reopening **ADR-004 §4.1** explicitly, as it
instructs, not a quiet flag change.

## Action Items

1. [x] Resolve the LGPL/H.264 contradiction before the pipeline is built around it.
2. [ ] Probe encoder availability at startup; fall back to `libopenh264`; report which is in use.
3. [ ] Produce the stripped build and verify it is ≤25 MB (ADR-004 §4.3).
4. [ ] Write `NOTICE` with licence text, pinned version, and the `configure` line.
5. [ ] Compare `h264_mf` and `libopenh264` output quality at 1080p30 in M2 and record numbers.
