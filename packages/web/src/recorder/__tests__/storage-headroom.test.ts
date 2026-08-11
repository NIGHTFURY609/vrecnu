import { afterEach, describe, expect, it, vi } from "vitest";
import {
  checkStorageHeadroom,
  ESTIMATED_BYTES_PER_SECOND,
  formatBytes,
  formatDurationApprox,
  requestPersistentStorage,
} from "../storage-headroom";

afterEach(() => {
  vi.unstubAllGlobals();
});

function stubStorage(storage: unknown): void {
  vi.stubGlobal("navigator", { storage });
}

describe("requestPersistentStorage", () => {
  it("reports unsupported when the API is missing", async () => {
    stubStorage({});
    expect(await requestPersistentStorage()).toBe("unsupported");
  });

  it("does not re-request when already persisted", async () => {
    const persist = vi.fn();
    stubStorage({ persisted: () => Promise.resolve(true), persist });
    expect(await requestPersistentStorage()).toBe("granted");
    expect(persist).not.toHaveBeenCalled();
  });

  it("reports denied when the browser refuses", async () => {
    stubStorage({ persisted: () => Promise.resolve(false), persist: () => Promise.resolve(false) });
    expect(await requestPersistentStorage()).toBe("denied");
  });

  it("reports granted when the browser agrees", async () => {
    stubStorage({ persisted: () => Promise.resolve(false), persist: () => Promise.resolve(true) });
    expect(await requestPersistentStorage()).toBe("granted");
  });

  it("treats a throwing implementation as unsupported rather than failing", async () => {
    stubStorage({
      persisted: () => {
        throw new Error("nope");
      },
      persist: () => Promise.resolve(true),
    });
    expect(await requestPersistentStorage()).toBe("unsupported");
  });
});

describe("checkStorageHeadroom", () => {
  it("flags low headroom when free space is under twice the projection", async () => {
    // 60s projects to 60 * 625 KB; give it exactly 1.5x, which is below the 2x safety factor.
    const projected = 60 * ESTIMATED_BYTES_PER_SECOND;
    stubStorage({ estimate: () => Promise.resolve({ quota: projected * 1.5, usage: 0 }) });

    const headroom = await checkStorageHeadroom(60);
    expect(headroom.supported).toBe(true);
    expect(headroom.low).toBe(true);
    expect(headroom.projectedBytes).toBe(projected);
  });

  it("does not flag when there is comfortable headroom", async () => {
    const projected = 60 * ESTIMATED_BYTES_PER_SECOND;
    stubStorage({ estimate: () => Promise.resolve({ quota: projected * 10, usage: 0 }) });
    expect((await checkStorageHeadroom(60)).low).toBe(false);
  });

  it("subtracts existing usage from the quota", async () => {
    stubStorage({ estimate: () => Promise.resolve({ quota: 1_000, usage: 400 }) });
    expect((await checkStorageHeadroom(1)).availableBytes).toBe(600);
  });

  it("never reports negative space when usage exceeds quota", async () => {
    stubStorage({ estimate: () => Promise.resolve({ quota: 100, usage: 500 }) });
    expect((await checkStorageHeadroom(1)).availableBytes).toBe(0);
  });

  it("derives a recordable duration from the free space", async () => {
    stubStorage({ estimate: () => Promise.resolve({ quota: ESTIMATED_BYTES_PER_SECOND * 120, usage: 0 }) });
    expect((await checkStorageHeadroom(1)).recordableSeconds).toBe(120);
  });

  it("reports unsupported, and never 'low', when estimate is missing", async () => {
    stubStorage({});
    const headroom = await checkStorageHeadroom(60);
    expect(headroom.supported).toBe(false);
    // An unknown quota must not produce a scary warning.
    expect(headroom.low).toBe(false);
  });

  it("reports unsupported when the browser omits the numbers", async () => {
    stubStorage({ estimate: () => Promise.resolve({}) });
    expect((await checkStorageHeadroom(60)).supported).toBe(false);
  });
});

describe("formatBytes", () => {
  it.each([
    [512, "512 B"],
    [1024, "1 KB"],
    [1536, "1.5 KB"],
    [1024 * 1024 * 3.5, "3.5 MB"],
    [1024 * 1024 * 1024 * 2, "2 GB"],
    [1024 * 1024 * 512, "512 MB"],
  ])("formats %i as %s", (input, expected) => {
    expect(formatBytes(input)).toBe(expected);
  });
});

describe("formatDurationApprox", () => {
  it.each([
    [45, "45s"],
    [90, "1 min"],
    [3_600, "1 hr"],
    [5_400, "1 hr 30 min"],
  ])("formats %i seconds as %s", (input, expected) => {
    expect(formatDurationApprox(input)).toBe(expected);
  });
});
