/**
 * Plans an **in-place** WebM duration patch (ADR-003 §3.2).
 *
 * Why this exists: `fix-webm-duration` reads the entire recording via
 * `FileReader.readAsArrayBuffer` and returns a fresh `Blob`, i.e. a full in-RAM copy
 * (transiently ~2x) of a file that may be gigabytes. That is exactly the accumulation
 * AGENTS.md §10 forbids and ADR-003 was written to eliminate — reintroduced at the last
 * moment, after capture had correctly kept memory flat the whole way through.
 *
 * The fix: MediaRecorder's WebM header arrives in the *first* chunk, a few kilobytes.
 * We rewrite only that header — inserting a `Duration` element into `Info` with a
 * placeholder — and then, at finalize, overwrite 8 bytes at a known offset. Peak memory
 * is the header, not the recording, and finalize is a single seek+write regardless of
 * file size.
 *
 * Chromium's live WebM muxer emits an unknown-size `Segment` with no `SeekHead` and no
 * `Cues`, which is what makes header insertion safe: there are no absolute byte offsets
 * elsewhere in the file to invalidate. If a `SeekHead` or `Cues` ever does appear, we
 * refuse to patch rather than silently corrupt the index — see `UnsupportedReason`.
 */

import {
  CLUSTER_ID,
  CUES_ID,
  DEFAULT_TIMECODE_SCALE_NS,
  DURATION_ID,
  EBML_HEADER_ID,
  INFO_ID,
  readElementHeader,
  readUintBody,
  SEEK_HEAD_ID,
  SEGMENT_ID,
  TIMECODE_SCALE_ID,
  writeFixedWidthSize,
} from "./webm-ebml";

/** Serialized `Duration`: ID `44 89`, size `88` (8), then a big-endian float64. */
const DURATION_ELEMENT_BYTES = 11;
const DURATION_VALUE_BYTES = 8;

export type UnsupportedReason =
  | "not-webm"
  | "no-segment"
  | "seek-index-present"
  | "info-missing"
  | "malformed";

export type DurationPatchPlan =
  | {
      /** `Info` already carries a `Duration`; nothing to rewrite, just overwrite the value. */
      readonly kind: "in-place";
      readonly valueOffset: number;
      readonly valueBytes: 4 | 8;
      readonly timecodeScaleNs: number;
    }
  | {
      /** A `Duration` must be inserted; `header` replaces the first `consumed` bytes of the file. */
      readonly kind: "rewrite-header";
      readonly header: Uint8Array;
      readonly consumed: number;
      readonly valueOffset: number;
      readonly valueBytes: 8;
      readonly timecodeScaleNs: number;
    };

export type DurationPatchResult =
  | { readonly status: "ready"; readonly plan: DurationPatchPlan }
  /** Not enough bytes buffered yet — feed more of the stream and retry. */
  | { readonly status: "incomplete" }
  | { readonly status: "unsupported"; readonly reason: UnsupportedReason };

function unsupported(reason: UnsupportedReason): DurationPatchResult {
  return { status: "unsupported", reason };
}

/**
 * Scans `Info`'s children for `TimecodeScale` and an existing `Duration`.
 * Returns `null` when the buffer ends mid-element.
 */
function scanInfo(
  bytes: Uint8Array,
  contentStart: number,
  contentEnd: number,
): { timecodeScaleNs: number; duration: { valueOffset: number; valueBytes: 4 | 8 } | null } | null {
  let timecodeScaleNs = DEFAULT_TIMECODE_SCALE_NS;
  let duration: { valueOffset: number; valueBytes: 4 | 8 } | null = null;
  let offset = contentStart;

  while (offset < contentEnd) {
    const child = readElementHeader(bytes, offset);
    if (!child || child.unknownSize) {
      return null;
    }
    const childEnd = child.contentStart + child.size;
    if (childEnd > bytes.length) {
      return null;
    }
    if (child.id === TIMECODE_SCALE_ID) {
      const scale = readUintBody(bytes, child.contentStart, child.size);
      if (scale !== null && scale > 0) {
        timecodeScaleNs = scale;
      }
    } else if (child.id === DURATION_ID && (child.size === 4 || child.size === 8)) {
      duration = { valueOffset: child.contentStart, valueBytes: child.size };
    }
    offset = childEnd;
  }

  return { timecodeScaleNs, duration };
}

/**
 * @param bytes A prefix of the recording — everything buffered so far.
 * @returns A plan, `incomplete` (buffer more and call again), or `unsupported`.
 */
export function planDurationPatch(bytes: Uint8Array): DurationPatchResult {
  const ebml = readElementHeader(bytes, 0);
  if (!ebml) {
    return { status: "incomplete" };
  }
  if (ebml.id !== EBML_HEADER_ID) {
    return unsupported("not-webm");
  }
  if (ebml.unknownSize) {
    return unsupported("malformed");
  }

  const segmentStart = ebml.contentStart + ebml.size;
  const segment = readElementHeader(bytes, segmentStart);
  if (!segment) {
    return { status: "incomplete" };
  }
  if (segment.id !== SEGMENT_ID) {
    return unsupported("no-segment");
  }

  // Walk the Segment's children looking for Info, bailing the moment we see something
  // that would make header insertion unsafe or pointless.
  let offset = segment.contentStart;
  let info: ReturnType<typeof readElementHeader> = null;

  while (info === null) {
    const child = readElementHeader(bytes, offset);
    if (!child) {
      return { status: "incomplete" };
    }
    if (child.id === SEEK_HEAD_ID || child.id === CUES_ID) {
      // These hold absolute byte positions within the Segment. Inserting bytes ahead of
      // them would silently invalidate every entry, so refuse instead.
      return unsupported("seek-index-present");
    }
    if (child.id === CLUSTER_ID) {
      // Media data has started; Info is never emitted after this point.
      return unsupported("info-missing");
    }
    if (child.id === INFO_ID) {
      info = child;
      break;
    }
    if (child.unknownSize) {
      return unsupported("malformed");
    }
    offset = child.contentStart + child.size;
  }

  if (info.unknownSize) {
    return unsupported("malformed");
  }
  const infoContentEnd = info.contentStart + info.size;
  if (infoContentEnd > bytes.length) {
    return { status: "incomplete" };
  }

  const scanned = scanInfo(bytes, info.contentStart, infoContentEnd);
  if (!scanned) {
    return { status: "incomplete" };
  }

  if (scanned.duration) {
    return {
      status: "ready",
      plan: {
        kind: "in-place",
        valueOffset: scanned.duration.valueOffset,
        valueBytes: scanned.duration.valueBytes,
        timecodeScaleNs: scanned.timecodeScaleNs,
      },
    };
  }

  // Rebuild the prefix with a Duration inserted at the head of Info's content. Both the
  // Segment and Info sizes are re-encoded at full 8-byte width so growth can never fail
  // to fit the original field.
  const header: number[] = [];
  const push = (source: Uint8Array, from: number, to: number): void => {
    for (let i = from; i < to; i++) {
      header.push(source[i] ?? 0);
    }
  };

  push(bytes, 0, segment.sizeOffset);
  if (segment.unknownSize) {
    push(bytes, segment.sizeOffset, segment.sizeOffset + segment.sizeLength);
  } else {
    const infoSizeGrowth = 8 - info.sizeLength + DURATION_ELEMENT_BYTES;
    const grown = writeFixedWidthSize(segment.size + infoSizeGrowth);
    for (const byte of grown) {
      header.push(byte);
    }
  }
  push(bytes, segment.contentStart, info.start);
  push(bytes, info.start, info.sizeOffset);
  for (const byte of writeFixedWidthSize(info.size + DURATION_ELEMENT_BYTES)) {
    header.push(byte);
  }
  header.push(0x44, 0x89, 0x88);
  const valueOffset = header.length;
  for (let i = 0; i < DURATION_VALUE_BYTES; i++) {
    header.push(0);
  }

  return {
    status: "ready",
    plan: {
      kind: "rewrite-header",
      header: Uint8Array.from(header),
      consumed: info.contentStart,
      valueOffset,
      valueBytes: 8,
      timecodeScaleNs: scanned.timecodeScaleNs,
    },
  };
}

/**
 * Encodes a wall-clock duration as the big-endian float Matroska expects — a count of
 * TimecodeScale ticks, not milliseconds.
 */
export function encodeDurationValue(durationMs: number, timecodeScaleNs: number, valueBytes: 4 | 8): Uint8Array {
  const ticks = (durationMs * 1_000_000) / timecodeScaleNs;
  const out = new Uint8Array(valueBytes);
  const view = new DataView(out.buffer);
  if (valueBytes === 8) {
    view.setFloat64(0, ticks, false);
  } else {
    view.setFloat32(0, ticks, false);
  }
  return out;
}
