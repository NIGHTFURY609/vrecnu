import { describe, expect, it, vi } from "vitest";
import { blobByteSource, probeRangeSourceSize, rangeByteSource } from "../byte-source";
import { GoogleDriveProvider } from "../google-drive-provider";
import { FakeDrive } from "./fake-drive";

describe("blobByteSource", () => {
  it("reports the blob's size", () => {
    expect(blobByteSource(new Blob([new Uint8Array(1234)])).size).toBe(1234);
  });

  it("slices the half-open range, matching Blob.slice", async () => {
    const source = blobByteSource(new Blob([Uint8Array.from([0, 1, 2, 3, 4, 5])]));
    const slice = await source.slice(2, 5);
    expect(Array.from(new Uint8Array(await slice.arrayBuffer()))).toEqual([2, 3, 4]);
  });
});

describe("rangeByteSource", () => {
  it("converts a half-open range into an inclusive HTTP Range header", async () => {
    const fetchImpl = vi.fn(async () => new Response(new Uint8Array([9, 9, 9]), { status: 206 }));
    const source = rangeByteSource("/r/tok", { size: 100, fetchImpl: fetchImpl as unknown as typeof fetch });

    await source.slice(0, 3);

    // Blob.slice(0, 3) is bytes 0,1,2 — so the header must be `bytes=0-2`, not `0-3`.
    expect(fetchImpl).toHaveBeenCalledWith("/r/tok", { headers: { Range: "bytes=0-2" } });
  });

  it("accepts a 200 for servers that ignore Range on a full-length request", async () => {
    const fetchImpl = vi.fn(async () => new Response(new Uint8Array([1]), { status: 200 }));
    const source = rangeByteSource("/r/tok", { size: 1, fetchImpl: fetchImpl as unknown as typeof fetch });
    await expect(source.slice(0, 1)).resolves.toBeInstanceOf(Blob);
  });

  it("throws on an unexpected status rather than uploading garbage", async () => {
    const fetchImpl = vi.fn(async () => new Response(null, { status: 403 }));
    const source = rangeByteSource("/r/tok", { size: 10, fetchImpl: fetchImpl as unknown as typeof fetch });
    await expect(source.slice(0, 5)).rejects.toThrow(/403/);
  });
});

describe("probeRangeSourceSize", () => {
  it("reads Content-Length from a HEAD", async () => {
    const fetchImpl = vi.fn(async () => new Response(null, { status: 200, headers: { "Content-Length": "4096" } }));
    await expect(probeRangeSourceSize("/r/tok", fetchImpl as unknown as typeof fetch)).resolves.toBe(4096);
    expect(fetchImpl).toHaveBeenCalledWith("/r/tok", { method: "HEAD" });
  });

  it("fails loudly when the server omits Content-Length", async () => {
    const fetchImpl = vi.fn(async () => new Response(null, { status: 200 }));
    await expect(probeRangeSourceSize("/r/tok", fetchImpl as unknown as typeof fetch)).rejects.toThrow(
      /Content-Length/,
    );
  });
});

describe("upload from a ranged source", () => {
  it("drives the same resumable loop as a Blob, reading only the window it needs", async () => {
    // This is the ADR-002 desktop path: bytes stay on disk behind the loopback server and
    // are pulled a window at a time. Proving the provider never asks for the whole file is
    // the point — `fetch(url).then(r => r.blob())` is what this design exists to avoid.
    const total = 20 * 1024 * 1024;
    const requestedRanges: string[] = [];
    const rangeFetch = (async (_url: string, init: RequestInit = {}) => {
      const range = (init.headers as Record<string, string>).Range ?? "";
      requestedRanges.push(range);
      const match = /bytes=(\d+)-(\d+)/.exec(range);
      const length = Number(match?.[2] ?? 0) - Number(match?.[1] ?? 0) + 1;
      return new Response(new Uint8Array(length), { status: 206 });
    }) as unknown as typeof fetch;

    const drive = new FakeDrive();
    const provider = new GoogleDriveProvider({
      getAccessToken: async () => "tok",
      fetchImpl: drive.fetch,
      sleep: async () => {},
    });

    const { fileId } = await provider.uploadResumable(
      rangeByteSource("/r/tok", { size: total, fetchImpl: rangeFetch }),
      { name: "clip.webm", mimeType: "video/webm" },
    );

    expect(fileId).toBe("file-complete");
    expect(drive.received).toBe(total);
    // 20 MiB in 8 MiB windows = three requests, none of them the whole file.
    expect(requestedRanges).toEqual([
      "bytes=0-8388607",
      "bytes=8388608-16777215",
      "bytes=16777216-20971519",
    ]);
  });
});
