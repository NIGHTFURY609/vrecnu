import { afterEach, describe, expect, it, vi } from "vitest";
import {
  buildRecordingFileName,
  deleteRecording,
  listRecordings,
  listRecoverableRecordings,
  totalRecordingBytes,
} from "../opfs-sessions";

interface FakeFile {
  name: string;
  size: number;
  lastModified: number;
  /** Simulates a file still held open by a live sync access handle. */
  locked?: boolean;
}

const removeEntry = vi.fn<(name: string) => Promise<void>>();

function stubOpfs(files: FakeFile[], options: { iterable?: boolean } = {}): void {
  const handles = files.map((file) => ({
    kind: "file" as const,
    name: file.name,
    getFile: () =>
      file.locked
        ? Promise.reject(new Error("NoModificationAllowedError"))
        : Promise.resolve({ size: file.size, lastModified: file.lastModified }),
  }));

  const directory: Record<string, unknown> = { removeEntry };
  if (options.iterable !== false) {
    directory.values = async function* () {
      // A directory can hold things that are not recordings; the scan must skip them.
      yield { kind: "directory" as const, name: "some-subdir" };
      for (const handle of handles) {
        yield handle;
      }
    };
  }

  vi.stubGlobal("navigator", { storage: { getDirectory: () => Promise.resolve(directory) } });
}

afterEach(() => {
  vi.unstubAllGlobals();
  removeEntry.mockReset();
});

describe("buildRecordingFileName", () => {
  it("encodes the container and start time", () => {
    expect(buildRecordingFileName("webm", 1_700_000_000_000)).toBe("vrecnu-1700000000000.webm");
    expect(buildRecordingFileName("mp4", 42)).toBe("vrecnu-42.mp4");
  });

  it("round-trips through listRecordings", async () => {
    const name = buildRecordingFileName("mp4", 12_345);
    stubOpfs([{ name, size: 10, lastModified: 1 }]);
    const [session] = await listRecordings();
    expect(session).toMatchObject({ fileName: name, container: "mp4", startedAt: 12_345, sizeBytes: 10 });
  });
});

describe("listRecordings", () => {
  it("returns recordings newest first and ignores unrelated entries", async () => {
    stubOpfs([
      { name: "vrecnu-100.webm", size: 5, lastModified: 1 },
      { name: "notes.txt", size: 5, lastModified: 1 },
      { name: "vrecnu-300.mp4", size: 7, lastModified: 2 },
      { name: "vrecnu-200.webm", size: 6, lastModified: 3 },
    ]);
    const names = (await listRecordings()).map((session) => session.fileName);
    expect(names).toEqual(["vrecnu-300.mp4", "vrecnu-200.webm", "vrecnu-100.webm"]);
  });

  it("skips a file that is still locked rather than failing the whole scan", async () => {
    stubOpfs([
      { name: "vrecnu-100.webm", size: 5, lastModified: 1, locked: true },
      { name: "vrecnu-200.webm", size: 6, lastModified: 2 },
    ]);
    const names = (await listRecordings()).map((session) => session.fileName);
    expect(names).toEqual(["vrecnu-200.webm"]);
  });

  it("returns empty when OPFS is unavailable", async () => {
    vi.stubGlobal("navigator", {});
    expect(await listRecordings()).toEqual([]);
  });

  it("returns empty when the directory handle is not iterable", async () => {
    stubOpfs([{ name: "vrecnu-100.webm", size: 5, lastModified: 1 }], { iterable: false });
    expect(await listRecordings()).toEqual([]);
  });
});

describe("listRecoverableRecordings", () => {
  it("excludes the session this page owns", async () => {
    stubOpfs([
      { name: "vrecnu-100.webm", size: 5, lastModified: 1 },
      { name: "vrecnu-200.webm", size: 6, lastModified: 2 },
    ]);
    const names = (await listRecoverableRecordings("vrecnu-200.webm")).map((s) => s.fileName);
    expect(names).toEqual(["vrecnu-100.webm"]);
  });

  it("drops zero-byte files, which died before the first chunk landed", async () => {
    stubOpfs([
      { name: "vrecnu-100.webm", size: 0, lastModified: 1 },
      { name: "vrecnu-200.webm", size: 6, lastModified: 2 },
    ]);
    const names = (await listRecoverableRecordings()).map((s) => s.fileName);
    expect(names).toEqual(["vrecnu-200.webm"]);
  });
});

describe("deleteRecording", () => {
  it("removes the entry", async () => {
    stubOpfs([]);
    removeEntry.mockResolvedValue(undefined);
    await deleteRecording("vrecnu-100.webm");
    expect(removeEntry).toHaveBeenCalledWith("vrecnu-100.webm");
  });

  it("swallows a failure — a locked or already-deleted file is not actionable", async () => {
    stubOpfs([]);
    removeEntry.mockRejectedValue(new Error("still open"));
    await expect(deleteRecording("vrecnu-100.webm")).resolves.toBeUndefined();
  });
});

describe("totalRecordingBytes", () => {
  it("sums every recording on disk", async () => {
    stubOpfs([
      { name: "vrecnu-100.webm", size: 5, lastModified: 1 },
      { name: "vrecnu-200.webm", size: 6, lastModified: 2 },
    ]);
    expect(await totalRecordingBytes()).toBe(11);
  });
});
