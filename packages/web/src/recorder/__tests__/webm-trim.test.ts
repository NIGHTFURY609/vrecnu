import { describe, expect, it } from "vitest";
import { buildTrimmedWebm, indexWebmClusters } from "../webm-trim";
import { sanitizeFileName } from "../download";

const EBML_HEADER = [0x1a, 0x45, 0xdf, 0xa3];
const SEGMENT = [0x18, 0x53, 0x80, 0x67];
const INFO = [0x15, 0x49, 0xa9, 0x66];
const TIMECODE_SCALE = [0x2a, 0xd7, 0xb1];
const TRACKS = [0x16, 0x54, 0xae, 0x6b];
const CLUSTER = [0x1f, 0x43, 0xb6, 0x75];
const CLUSTER_TIMECODE = [0xe7];

function shortSize(n: number): number[] {
  if (n > 0x7e) {
    throw new Error(`shortSize only encodes < 127, got ${n}`);
  }
  return [0x80 | n];
}

function elem(id: number[], content: number[]): number[] {
  return [...id, ...shortSize(content.length), ...content];
}

/** Two-byte big-endian, so a rebased timecode still fits the original element width. */
function u16(value: number): number[] {
  return [(value >> 8) & 0xff, value & 0xff];
}

/** One cluster: Timecode + a recognisable payload so we can prove the bytes survive. */
function cluster(timecodeTicks: number, payload: number[]): number[] {
  const content = [...elem(CLUSTER_TIMECODE, u16(timecodeTicks)), ...payload];
  return elem(CLUSTER, content);
}

function buildFixture(timecodes: number[]): Blob {
  const info = elem(INFO, elem(TIMECODE_SCALE, [0x0f, 0x42, 0x40])); // 1_000_000 ns = 1 ms
  const tracks = elem(TRACKS, [0x01, 0x02]);
  const clusters = timecodes.flatMap((tc, i) => cluster(tc, [0xa0 + i, 0xb0 + i, 0xc0 + i]));
  return new Blob([
    Uint8Array.from([
      ...elem(EBML_HEADER, [0x42, 0x86, 0x81, 0x01]),
      ...SEGMENT,
      0xff, // unknown Segment size, as a live mux emits
      ...info,
      ...tracks,
      ...clusters,
    ]),
  ]);
}

describe("indexWebmClusters", () => {
  it("indexes every cluster with its timecode", async () => {
    const result = await indexWebmClusters(buildFixture([0, 1000, 2000, 3000, 4000]));
    if (!result.ok) {
      throw new Error(`expected an index, got ${result.reason}`);
    }
    expect(result.index.clusters.map((c) => c.timecodeTicks)).toEqual([0, 1000, 2000, 3000, 4000]);
    expect(result.index.timecodeScaleNs).toBe(1_000_000);
    expect(result.index.durationMs).toBe(4000);
  });

  it("places headerEnd at the first cluster", async () => {
    const result = await indexWebmClusters(buildFixture([0, 1000]));
    if (!result.ok) {
      throw new Error("expected an index");
    }
    expect(result.index.headerEnd).toBe(result.index.clusters[0]?.offset);
  });

  it("rejects a file that is not WebM", async () => {
    const result = await indexWebmClusters(new Blob([Uint8Array.from([0, 0, 0, 0x20, 0x66, 0x74, 0x79, 0x70])]));
    expect(result).toEqual({ ok: false, reason: "not-webm" });
  });

  it("refuses rather than guessing when a cluster declares an unknown size", async () => {
    const info = elem(INFO, elem(TIMECODE_SCALE, [0x0f, 0x42, 0x40]));
    const bytes = Uint8Array.from([
      ...elem(EBML_HEADER, [0x42, 0x86, 0x81, 0x01]),
      ...SEGMENT,
      0xff,
      ...info,
      ...CLUSTER,
      0xff, // unknown cluster size
      ...elem(CLUSTER_TIMECODE, u16(0)),
      0xaa,
    ]);
    expect(await indexWebmClusters(new Blob([bytes]))).toEqual({ ok: false, reason: "unknown-cluster-size" });
  });

  it("reports no-clusters for a header-only file", async () => {
    const info = elem(INFO, elem(TIMECODE_SCALE, [0x0f, 0x42, 0x40]));
    const bytes = Uint8Array.from([...elem(EBML_HEADER, [0x42, 0x86, 0x81, 0x01]), ...SEGMENT, 0xff, ...info]);
    expect(await indexWebmClusters(new Blob([bytes]))).toEqual({ ok: false, reason: "no-clusters" });
  });
});

describe("buildTrimmedWebm", () => {
  const timecodes = [0, 1000, 2000, 3000, 4000];

  it("keeps the cluster containing the start point, not the one after it", async () => {
    const file = buildFixture(timecodes);
    const indexed = await indexWebmClusters(file);
    if (!indexed.ok) {
      throw new Error("expected an index");
    }

    // 1500 ms falls inside the cluster that starts at 1000; dropping it would silently lose
    // half a second of video the user asked to keep.
    const result = await buildTrimmedWebm(file, indexed.index, { startMs: 1500, endMs: 3500 }, "video/webm");
    expect(result?.clustersKept).toBe(3);
    expect(result?.actualStartMs).toBe(1000);
  });

  it("rebases kept cluster timecodes to zero", async () => {
    const file = buildFixture(timecodes);
    const indexed = await indexWebmClusters(file);
    if (!indexed.ok) {
      throw new Error("expected an index");
    }
    const result = await buildTrimmedWebm(file, indexed.index, { startMs: 2000, endMs: 4000 }, "video/webm");
    if (!result) {
      throw new Error("expected a trimmed blob");
    }

    // Re-indexing the output is the real proof: it only parses if the rewritten clusters are
    // structurally valid, and the timecodes must now start at zero.
    const reindexed = await indexWebmClusters(result.blob);
    if (!reindexed.ok) {
      throw new Error(`trimmed output failed to re-index: ${reindexed.reason}`);
    }
    expect(reindexed.index.clusters.map((c) => c.timecodeTicks)).toEqual([0, 1000, 2000]);
  });

  it("preserves cluster payload bytes exactly", async () => {
    const file = buildFixture(timecodes);
    const indexed = await indexWebmClusters(file);
    if (!indexed.ok) {
      throw new Error("expected an index");
    }
    const result = await buildTrimmedWebm(file, indexed.index, { startMs: 3000, endMs: 4000 }, "video/webm");
    if (!result) {
      throw new Error("expected a trimmed blob");
    }
    const bytes = new Uint8Array(await result.blob.arrayBuffer());
    // Clusters 3 and 4 carry 0xa3,0xb3,0xc3 and 0xa4,0xb4,0xc4.
    expect(Array.from(bytes.subarray(bytes.length - 3))).toEqual([0xa4, 0xb4, 0xc4]);
    expect(Array.from(bytes).join(",")).toContain([0xa3, 0xb3, 0xc3].join(","));
  });

  it("drops clusters after the end point", async () => {
    const file = buildFixture(timecodes);
    const indexed = await indexWebmClusters(file);
    if (!indexed.ok) {
      throw new Error("expected an index");
    }
    const result = await buildTrimmedWebm(file, indexed.index, { startMs: 0, endMs: 1500 }, "video/webm");
    expect(result?.clustersKept).toBe(2);
  });

  it("produces a smaller file than the original", async () => {
    const file = buildFixture(timecodes);
    const indexed = await indexWebmClusters(file);
    if (!indexed.ok) {
      throw new Error("expected an index");
    }
    const result = await buildTrimmedWebm(file, indexed.index, { startMs: 3000, endMs: 4000 }, "video/webm");
    expect(result!.blob.size).toBeLessThan(file.size);
  });

  it("returns null when the selection keeps nothing", async () => {
    const file = buildFixture(timecodes);
    const indexed = await indexWebmClusters(file);
    if (!indexed.ok) {
      throw new Error("expected an index");
    }
    const empty = { ...indexed.index, clusters: [] };
    expect(await buildTrimmedWebm(file, empty, { startMs: 0, endMs: 1000 }, "video/webm")).toBeNull();
  });
});

describe("sanitizeFileName", () => {
  it("keeps spaces and hyphens, which are legal on every target platform", () => {
    expect(sanitizeFileName("my recording - take 2", "fallback")).toBe("my recording - take 2");
  });

  it("strips characters Windows rejects", () => {
    expect(sanitizeFileName('a<b>c:d"e/f\\g|h?i*j', "fallback")).toBe("abcdefghij");
  });

  it("falls back when nothing usable remains", () => {
    expect(sanitizeFileName("///", "vrecnu-recording")).toBe("vrecnu-recording");
    expect(sanitizeFileName("   ", "vrecnu-recording")).toBe("vrecnu-recording");
  });

  it("strips leading dots so the file is not hidden", () => {
    expect(sanitizeFileName("...secret", "fallback")).toBe("secret");
  });

  it("caps the length", () => {
    expect(sanitizeFileName("a".repeat(500), "fallback")).toHaveLength(120);
  });
});
