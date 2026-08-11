/**
 * Lossless WebM trimming by Cluster boundary.
 *
 * Phase 1 is allowed "basic trim only" and explicitly **no FFmpeg** (AGENTS.md §2, §10), so
 * re-encoding is off the table. The naive alternative — slicing the file at arbitrary byte
 * offsets — produces an unplayable file: cut the front and you lose the EBML header and
 * Tracks; cut the back and you truncate a Cluster mid-block.
 *
 * What actually works is a container-aware cut:
 *   header (EBML + Segment + Info + Tracks)  ++  the Clusters inside the range
 * with each kept Cluster's Timecode rebased to the new zero, and Info/Duration rewritten.
 * That is lossless, needs no decoding, and runs in milliseconds regardless of file size.
 *
 * Everything is assembled from `Blob.slice()` refs, never `ArrayBuffer`s, so a 4 GB recording
 * is trimmed without materialising a single frame in RAM — the same constraint ADR-003 puts
 * on capture applies just as much to editing.
 *
 * Cut points land on Cluster boundaries, which in MediaRecorder output are keyframes. The
 * result is a trim accurate to roughly the cluster interval rather than the exact frame; that
 * is the honest cost of not re-encoding, and the UI reports the snapped values back.
 */

import {
  CLUSTER_ID,
  DEFAULT_TIMECODE_SCALE_NS,
  readElementHeader,
  readUintBody,
  SEGMENT_ID,
  TIMECODE_SCALE_ID,
} from "./webm-ebml";
import { encodeDurationValue, planDurationPatch } from "./webm-duration";

/** Cluster Timecode element ID. */
const CLUSTER_TIMECODE_ID = 0xe7;

/** Enough to hold the EBML header, Segment header, Info and Tracks in any real recording. */
const HEADER_WINDOW_BYTES = 1024 * 1024;

/** Enough to cover a Cluster's own header plus its leading Timecode element. */
const CLUSTER_HEADER_WINDOW_BYTES = 64;

export interface WebmCluster {
  /** Absolute offset of the Cluster element's first ID byte. */
  readonly offset: number;
  /** Total byte length of the Cluster element, header included. */
  readonly length: number;
  /** Cluster start time, in TimecodeScale ticks. */
  readonly timecodeTicks: number;
  /** Absolute offset of the Timecode element's value bytes. */
  readonly timecodeValueOffset: number;
  readonly timecodeValueLength: number;
}

export interface WebmIndex {
  /** Byte offset where the first Cluster begins — everything before it is the header. */
  readonly headerEnd: number;
  readonly timecodeScaleNs: number;
  readonly clusters: readonly WebmCluster[];
  readonly durationMs: number;
}

export type IndexFailure =
  | "not-webm"
  | "no-clusters"
  /** A Cluster declared an unknown size; finding the next one would mean guessing. */
  | "unknown-cluster-size"
  | "malformed";

export type IndexResult = { ok: true; index: WebmIndex } | { ok: false; reason: IndexFailure };

async function readWindow(file: Blob, start: number, length: number): Promise<Uint8Array> {
  const slice = file.slice(start, Math.min(start + length, file.size));
  return new Uint8Array(await slice.arrayBuffer());
}

function ticksToMs(ticks: number, timecodeScaleNs: number): number {
  return (ticks * timecodeScaleNs) / 1_000_000;
}

function msToTicks(ms: number, timecodeScaleNs: number): number {
  return Math.round((ms * 1_000_000) / timecodeScaleNs);
}

/**
 * Walks the file reading only element headers — never Cluster payloads. Cost is one small
 * read per Cluster, so indexing a long recording is I/O-trivial and RAM-flat.
 */
export async function indexWebmClusters(file: Blob): Promise<IndexResult> {
  const head = await readWindow(file, 0, HEADER_WINDOW_BYTES);

  const ebml = readElementHeader(head, 0);
  if (!ebml || ebml.unknownSize) {
    return { ok: false, reason: "not-webm" };
  }
  const segment = readElementHeader(head, ebml.contentStart + ebml.size);
  if (!segment || segment.id !== SEGMENT_ID) {
    return { ok: false, reason: "not-webm" };
  }

  // Scan Segment children for TimecodeScale and the first Cluster.
  let timecodeScaleNs = DEFAULT_TIMECODE_SCALE_NS;
  let offset = segment.contentStart;
  let headerEnd = -1;

  while (offset < head.length) {
    const child = readElementHeader(head, offset);
    if (!child) {
      break;
    }
    if (child.id === CLUSTER_ID) {
      headerEnd = child.start;
      break;
    }
    if (child.unknownSize) {
      return { ok: false, reason: "malformed" };
    }
    if (child.id === 0x1549a966) {
      // Info: pull TimecodeScale out of it.
      let infoOffset = child.contentStart;
      const infoEnd = Math.min(child.contentStart + child.size, head.length);
      while (infoOffset < infoEnd) {
        const infoChild = readElementHeader(head, infoOffset);
        if (!infoChild || infoChild.unknownSize) {
          break;
        }
        if (infoChild.id === TIMECODE_SCALE_ID) {
          const scale = readUintBody(head, infoChild.contentStart, infoChild.size);
          if (scale !== null && scale > 0) {
            timecodeScaleNs = scale;
          }
        }
        infoOffset = infoChild.contentStart + infoChild.size;
      }
    }
    offset = child.contentStart + child.size;
  }

  if (headerEnd < 0) {
    return { ok: false, reason: "no-clusters" };
  }

  const clusters: WebmCluster[] = [];
  let cursor = headerEnd;

  while (cursor < file.size) {
    const window = await readWindow(file, cursor, CLUSTER_HEADER_WINDOW_BYTES);
    const cluster = readElementHeader(window, 0);
    if (!cluster || cluster.id !== CLUSTER_ID) {
      break;
    }
    if (cluster.unknownSize) {
      // Locating the next Cluster would mean byte-scanning for an ID that can also occur
      // inside frame payloads. Refusing beats silently producing a corrupt trim.
      return { ok: false, reason: "unknown-cluster-size" };
    }

    const timecode = readElementHeader(window, cluster.contentStart);
    if (!timecode || timecode.id !== CLUSTER_TIMECODE_ID) {
      return { ok: false, reason: "malformed" };
    }
    const ticks = readUintBody(window, timecode.contentStart, timecode.size);
    if (ticks === null) {
      return { ok: false, reason: "malformed" };
    }

    const length = cluster.contentStart + cluster.size - cluster.start;
    clusters.push({
      offset: cursor,
      length,
      timecodeTicks: ticks,
      timecodeValueOffset: cursor + timecode.contentStart,
      timecodeValueLength: timecode.size,
    });
    cursor += length;
  }

  if (clusters.length === 0) {
    return { ok: false, reason: "no-clusters" };
  }

  const last = clusters[clusters.length - 1];
  return {
    ok: true,
    index: {
      headerEnd,
      timecodeScaleNs,
      clusters,
      durationMs: last ? ticksToMs(last.timecodeTicks, timecodeScaleNs) : 0,
    },
  };
}

/** Big-endian uint of a fixed width — preserving width keeps the element size unchanged. */
function encodeUintFixedWidth(value: number, width: number): Uint8Array {
  const out = new Uint8Array(width);
  let remaining = Math.max(0, Math.round(value));
  for (let i = width - 1; i >= 0; i--) {
    out[i] = remaining % 0x100;
    remaining = Math.floor(remaining / 0x100);
  }
  return out;
}

export interface TrimSelection {
  readonly startMs: number;
  readonly endMs: number;
}

export interface TrimResult {
  readonly blob: Blob;
  /** Where the cut actually landed after snapping to Cluster boundaries. */
  readonly actualStartMs: number;
  readonly actualEndMs: number;
  readonly clustersKept: number;
}

/**
 * Builds the trimmed file. Returns `null` when the selection keeps nothing.
 *
 * The returned Blob is a list of slice references plus a few tiny patched buffers; no frame
 * data is ever read into memory.
 */
export async function buildTrimmedWebm(
  file: Blob,
  index: WebmIndex,
  selection: TrimSelection,
  mimeType: string,
): Promise<TrimResult | null> {
  const startTicks = msToTicks(selection.startMs, index.timecodeScaleNs);
  const endTicks = msToTicks(selection.endMs, index.timecodeScaleNs);

  // Include the cluster that *contains* startMs, not the first one at or after it — otherwise
  // the trim silently drops up to one cluster of video the user asked to keep.
  let firstIndex = 0;
  for (let i = 0; i < index.clusters.length; i++) {
    const cluster = index.clusters[i];
    if (cluster && cluster.timecodeTicks <= startTicks) {
      firstIndex = i;
    } else {
      break;
    }
  }

  const kept: WebmCluster[] = [];
  for (let i = firstIndex; i < index.clusters.length; i++) {
    const cluster = index.clusters[i];
    if (!cluster) {
      continue;
    }
    if (cluster.timecodeTicks > endTicks) {
      break;
    }
    kept.push(cluster);
  }

  if (kept.length === 0) {
    return null;
  }

  const base = kept[0];
  if (!base) {
    return null;
  }
  const baseTicks = base.timecodeTicks;

  // Rewrite the header's Duration to the trimmed length.
  const headerBytes = new Uint8Array(await file.slice(0, index.headerEnd).arrayBuffer());
  const durationMs = ticksToMs(endTicks - baseTicks, index.timecodeScaleNs);
  const parts: BlobPart[] = [];

  const plan = planDurationPatch(headerBytes);
  if (plan.status === "ready" && plan.plan.kind === "rewrite-header") {
    const patchedHeader = new Uint8Array(
      plan.plan.header.length + (headerBytes.length - plan.plan.consumed),
    );
    patchedHeader.set(plan.plan.header, 0);
    patchedHeader.set(headerBytes.subarray(plan.plan.consumed), plan.plan.header.length);
    patchedHeader.set(
      encodeDurationValue(durationMs, plan.plan.timecodeScaleNs, plan.plan.valueBytes),
      plan.plan.valueOffset,
    );
    parts.push(patchedHeader);
  } else if (plan.status === "ready" && plan.plan.kind === "in-place") {
    const patchedHeader = headerBytes.slice();
    patchedHeader.set(
      encodeDurationValue(durationMs, plan.plan.timecodeScaleNs, plan.plan.valueBytes),
      plan.plan.valueOffset,
    );
    parts.push(patchedHeader);
  } else {
    // No Duration slot available; the header still works, the player just re-derives length.
    parts.push(headerBytes);
  }

  for (const cluster of kept) {
    const headerLength = cluster.timecodeValueOffset + cluster.timecodeValueLength - cluster.offset;
    const clusterHeader = new Uint8Array(
      await file.slice(cluster.offset, cluster.offset + headerLength).arrayBuffer(),
    );
    // Rebase the timecode in place, keeping the original byte width so the Cluster's declared
    // size stays correct — a uint element may carry leading zeros, which makes this safe.
    clusterHeader.set(
      encodeUintFixedWidth(cluster.timecodeTicks - baseTicks, cluster.timecodeValueLength),
      cluster.timecodeValueOffset - cluster.offset,
    );
    parts.push(clusterHeader);
    parts.push(file.slice(cluster.offset + headerLength, cluster.offset + cluster.length));
  }

  const lastKept = kept[kept.length - 1];
  return {
    blob: new Blob(parts, { type: mimeType }),
    actualStartMs: ticksToMs(baseTicks, index.timecodeScaleNs),
    actualEndMs: lastKept ? ticksToMs(lastKept.timecodeTicks, index.timecodeScaleNs) : durationMs,
    clustersKept: kept.length,
  };
}
