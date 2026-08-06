import { describe, expect, it } from "vitest";
import { encodeDurationValue, planDurationPatch } from "../webm-duration";
import { DEFAULT_TIMECODE_SCALE_NS, readElementHeader, readElementSize, writeFixedWidthSize } from "../webm-ebml";

const EBML_HEADER = [0x1a, 0x45, 0xdf, 0xa3];
const SEGMENT = [0x18, 0x53, 0x80, 0x67];
const SEEK_HEAD = [0x11, 0x4d, 0x9b, 0x74];
const INFO = [0x15, 0x49, 0xa9, 0x66];
const TIMECODE_SCALE = [0x2a, 0xd7, 0xb1];
const DURATION = [0x44, 0x89];
const CLUSTER = [0x1f, 0x43, 0xb6, 0x75];
const VOID = [0xec];

/** Size VINT, 1-byte form. Only valid below 127, which covers every fixture here. */
function shortSize(n: number): number[] {
  if (n > 0x7e) {
    throw new Error(`shortSize only encodes < 127, got ${n}`);
  }
  return [0x80 | n];
}

function elem(id: number[], content: number[]): number[] {
  return [...id, ...shortSize(content.length), ...content];
}

/** Segment with an unknown size — what a live MediaRecorder mux actually emits. */
function unknownSizeSegment(children: number[], wide = false): number[] {
  const size = wide ? [0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff] : [0xff];
  return [...SEGMENT, ...size, ...children];
}

function timecodeScale(ns: number): number[] {
  const bytes: number[] = [];
  let remaining = ns;
  while (remaining > 0) {
    bytes.unshift(remaining % 0x100);
    remaining = Math.floor(remaining / 0x100);
  }
  return elem(TIMECODE_SCALE, bytes);
}

const EBML_PREFIX = elem(EBML_HEADER, [0x42, 0x86, 0x81, 0x01]);

/** Chromium-shaped header: EBML, unknown-size Segment, Info without a Duration, then a Cluster. */
function chromiumStyleHeader(options: { wideSegment?: boolean; extraInfo?: number[] } = {}): Uint8Array {
  const info = elem(INFO, [...timecodeScale(DEFAULT_TIMECODE_SCALE_NS), ...(options.extraInfo ?? [])]);
  const cluster = elem(CLUSTER, [0x01, 0x02, 0x03]);
  return Uint8Array.from([...EBML_PREFIX, ...unknownSizeSegment([...info, ...cluster], options.wideSegment)]);
}

describe("readElementSize", () => {
  it("strips the marker bit from the value", () => {
    expect(readElementSize(Uint8Array.from([0x88]), 0)).toEqual({ value: 8, length: 1, unknown: false });
  });

  it("recognises the 1-byte unknown size", () => {
    expect(readElementSize(Uint8Array.from([0xff]), 0)?.unknown).toBe(true);
  });

  it("recognises the 8-byte unknown size Chromium emits for a live Segment", () => {
    const bytes = Uint8Array.from([0x01, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff]);
    expect(readElementSize(bytes, 0)?.unknown).toBe(true);
  });

  it("round-trips through writeFixedWidthSize", () => {
    const encoded = writeFixedWidthSize(300);
    const decoded = readElementSize(encoded, 0);
    expect(decoded).toEqual({ value: 300, length: 8, unknown: false });
  });
});

describe("planDurationPatch", () => {
  it("plans a header rewrite when Info carries no Duration", () => {
    const result = planDurationPatch(chromiumStyleHeader());
    expect(result.status).toBe("ready");
    if (result.status !== "ready" || result.plan.kind !== "rewrite-header") {
      throw new Error("expected a rewrite-header plan");
    }
    expect(result.plan.timecodeScaleNs).toBe(DEFAULT_TIMECODE_SCALE_NS);
    expect(result.plan.valueBytes).toBe(8);
  });

  it("produces a rewritten header that re-parses with the Duration in place", () => {
    const original = chromiumStyleHeader();
    const result = planDurationPatch(original);
    if (result.status !== "ready" || result.plan.kind !== "rewrite-header") {
      throw new Error("expected a rewrite-header plan");
    }
    const { header, consumed, valueOffset } = result.plan;

    // Reassemble the file exactly as the worker does, then fill the reserved slot.
    const patched = new Uint8Array(header.length + (original.length - consumed));
    patched.set(header, 0);
    patched.set(original.subarray(consumed), header.length);
    patched.set(encodeDurationValue(12_345, DEFAULT_TIMECODE_SCALE_NS, 8), valueOffset);

    // Re-planning the patched bytes must now find an existing Duration at the same offset —
    // this is what proves the rewritten header is structurally valid EBML, not just plausible.
    const replanned = planDurationPatch(patched);
    if (replanned.status !== "ready" || replanned.plan.kind !== "in-place") {
      throw new Error(`expected an in-place plan, got ${JSON.stringify(replanned)}`);
    }
    expect(replanned.plan.valueOffset).toBe(valueOffset);

    const view = new DataView(patched.buffer, patched.byteOffset, patched.byteLength);
    expect(view.getFloat64(valueOffset, false)).toBeCloseTo(12_345, 6);
  });

  it("keeps the trailing Cluster bytes intact across the rewrite", () => {
    const original = chromiumStyleHeader();
    const result = planDurationPatch(original);
    if (result.status !== "ready" || result.plan.kind !== "rewrite-header") {
      throw new Error("expected a rewrite-header plan");
    }
    const tail = original.subarray(result.plan.consumed);
    // The Cluster is the last thing in the fixture; its payload must survive byte-for-byte.
    expect(Array.from(tail.subarray(tail.length - 3))).toEqual([0x01, 0x02, 0x03]);
  });

  it("handles the 8-byte unknown Segment size", () => {
    const result = planDurationPatch(chromiumStyleHeader({ wideSegment: true }));
    expect(result.status).toBe("ready");
  });

  it("reports in-place when a Duration already exists", () => {
    const durationElem = [...DURATION, 0x88, 0, 0, 0, 0, 0, 0, 0, 0];
    const info = elem(INFO, [...timecodeScale(DEFAULT_TIMECODE_SCALE_NS), ...durationElem]);
    const bytes = Uint8Array.from([...EBML_PREFIX, ...unknownSizeSegment(info)]);

    const result = planDurationPatch(bytes);
    if (result.status !== "ready" || result.plan.kind !== "in-place") {
      throw new Error("expected an in-place plan");
    }
    expect(result.plan.valueBytes).toBe(8);
    expect(bytes[result.plan.valueOffset - 1]).toBe(0x88);
  });

  it("reads a non-default TimecodeScale", () => {
    const info = elem(INFO, timecodeScale(500_000));
    const bytes = Uint8Array.from([...EBML_PREFIX, ...unknownSizeSegment([...info, ...elem(CLUSTER, [])])]);
    const result = planDurationPatch(bytes);
    if (result.status !== "ready") {
      throw new Error("expected a plan");
    }
    expect(result.plan.timecodeScaleNs).toBe(500_000);
  });

  it("skips past a Void element preceding Info", () => {
    const info = elem(INFO, timecodeScale(DEFAULT_TIMECODE_SCALE_NS));
    const bytes = Uint8Array.from([
      ...EBML_PREFIX,
      ...unknownSizeSegment([...elem(VOID, [0, 0, 0, 0]), ...info, ...elem(CLUSTER, [])]),
    ]);
    expect(planDurationPatch(bytes).status).toBe("ready");
  });

  it("refuses to patch when a SeekHead would be invalidated", () => {
    const info = elem(INFO, timecodeScale(DEFAULT_TIMECODE_SCALE_NS));
    const bytes = Uint8Array.from([
      ...EBML_PREFIX,
      ...unknownSizeSegment([...elem(SEEK_HEAD, [0x01, 0x02]), ...info]),
    ]);
    expect(planDurationPatch(bytes)).toEqual({ status: "unsupported", reason: "seek-index-present" });
  });

  it("gives up once media data has started", () => {
    const bytes = Uint8Array.from([...EBML_PREFIX, ...unknownSizeSegment(elem(CLUSTER, [0x01]))]);
    expect(planDurationPatch(bytes)).toEqual({ status: "unsupported", reason: "info-missing" });
  });

  it("rejects a stream that is not EBML at all", () => {
    const bytes = Uint8Array.from([0x66, 0x74, 0x79, 0x70, 0x69, 0x73, 0x6f, 0x6d]);
    expect(planDurationPatch(bytes)).toEqual({ status: "unsupported", reason: "not-webm" });
  });

  it("asks for more bytes when the buffer ends mid-Info", () => {
    const full = chromiumStyleHeader();
    // Cut inside Info's content: past the Segment header, before Info is complete.
    const truncated = full.subarray(0, full.length - 12);
    expect(planDurationPatch(truncated).status).toBe("incomplete");
  });

  it("asks for more bytes on an empty buffer", () => {
    expect(planDurationPatch(new Uint8Array(0)).status).toBe("incomplete");
  });
});

describe("encodeDurationValue", () => {
  it("expresses milliseconds as TimecodeScale ticks", () => {
    const bytes = encodeDurationValue(2_000, DEFAULT_TIMECODE_SCALE_NS, 8);
    const view = new DataView(bytes.buffer);
    // Default scale is 1 ms per tick, so 2000 ms is 2000 ticks.
    expect(view.getFloat64(0, false)).toBe(2_000);
  });

  it("scales when TimecodeScale is not the 1 ms default", () => {
    const bytes = encodeDurationValue(2_000, 500_000, 8);
    // Half-millisecond ticks means twice as many of them.
    expect(new DataView(bytes.buffer).getFloat64(0, false)).toBe(4_000);
  });

  it("supports the 4-byte float form", () => {
    const bytes = encodeDurationValue(1_500, DEFAULT_TIMECODE_SCALE_NS, 4);
    expect(bytes.length).toBe(4);
    expect(new DataView(bytes.buffer).getFloat32(0, false)).toBe(1_500);
  });
});

describe("readElementHeader", () => {
  it("locates content past a multi-byte ID and size", () => {
    const bytes = Uint8Array.from(elem(INFO, [0xaa, 0xbb]));
    const header = readElementHeader(bytes, 0);
    expect(header?.contentStart).toBe(5);
    expect(header?.size).toBe(2);
    expect(header?.unknownSize).toBe(false);
  });
});
