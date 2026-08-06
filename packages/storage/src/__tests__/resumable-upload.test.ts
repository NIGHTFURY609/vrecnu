import { describe, expect, it, vi } from "vitest";
import { GoogleDriveProvider, parseRangeEnd } from "../google-drive-provider";
import { UploadError, type ResumableSession } from "../storage-provider";
import { FakeDrive, type FakeDriveOptions } from "./fake-drive";

/** 20 MiB — three 8 MiB chunks, so multi-chunk behaviour is actually exercised. */
const FILE_SIZE = 20 * 1024 * 1024;

function makeFile(size = FILE_SIZE): Blob {
  return new Blob([new Uint8Array(size)], { type: "video/webm" });
}

function makeProvider(drive: FakeDrive, overrides: Record<string, unknown> = {}) {
  return new GoogleDriveProvider({
    getAccessToken: async () => "fake-token",
    fetchImpl: drive.fetch,
    // No real waiting: backoff timing is not what these tests are about.
    sleep: async () => {},
    ...overrides,
  });
}

function setup(options: FakeDriveOptions = {}) {
  const drive = new FakeDrive(options);
  return { drive, provider: makeProvider(drive) };
}

describe("parseRangeEnd", () => {
  it("returns the next offset, not the last stored byte", () => {
    // Drive's `bytes=0-262143` means 262144 bytes are stored, so resume starts at 262144.
    expect(parseRangeEnd("bytes=0-262143")).toBe(262144);
  });

  it("treats a missing header as nothing stored", () => {
    expect(parseRangeEnd(null)).toBe(0);
    expect(parseRangeEnd("")).toBe(0);
  });

  it("ignores a malformed header rather than guessing", () => {
    expect(parseRangeEnd("bytes=garbage")).toBe(0);
  });
});

describe("resumable upload", () => {
  it("uploads a multi-chunk file to completion", async () => {
    const { drive, provider } = setup();
    const { fileId } = await provider.uploadResumable(makeFile(), { name: "a.webm", mimeType: "video/webm" });
    expect(fileId).toBe("file-complete");
    expect(drive.received).toBe(FILE_SIZE);
  });

  it("reports monotonic progress that ends at the full size", async () => {
    const { provider } = setup();
    const seen: number[] = [];
    await provider.uploadResumable(
      makeFile(),
      { name: "a.webm", mimeType: "video/webm" },
      { onProgress: (p) => seen.push(p.bytesUploaded) },
    );
    expect(seen[seen.length - 1]).toBe(FILE_SIZE);
    expect([...seen].sort((a, b) => a - b)).toEqual(seen);
  });

  it("hands the session to the caller before any bytes move, so it can be persisted", async () => {
    const { provider } = setup();
    const sessions: ResumableSession[] = [];
    await provider.uploadResumable(
      makeFile(),
      { name: "clip.webm", mimeType: "video/webm" },
      { onSessionStart: (s) => sessions.push(s) },
    );
    expect(sessions).toHaveLength(1);
    expect(sessions[0]).toMatchObject({ fileName: "clip.webm", fileSize: FILE_SIZE, mimeType: "video/webm" });
  });

  it("signals when the session is finished so the caller can forget it", async () => {
    const { provider } = setup();
    const ended = vi.fn();
    await provider.uploadResumable(
      makeFile(),
      { name: "a.webm", mimeType: "video/webm" },
      { onSessionEnd: ended },
    );
    expect(ended).toHaveBeenCalledTimes(1);
  });

  it("places the file in a folder when one is supplied", async () => {
    const { drive, provider } = setup();
    await provider.uploadResumable(makeFile(1024), {
      name: "a.webm",
      mimeType: "video/webm",
      folderId: "folder-1",
    });
    expect(drive.lastCreateBody).toMatchObject({ name: "a.webm", parents: ["folder-1"] });
  });

  it("omits parents entirely when no folder is supplied", async () => {
    const { drive, provider } = setup();
    await provider.uploadResumable(makeFile(1024), { name: "a.webm", mimeType: "video/webm" });
    expect(drive.lastCreateBody).not.toHaveProperty("parents");
  });
});

describe("resume after failure", () => {
  it("survives a dropped connection and finishes the upload", async () => {
    // The single most likely thing to happen during a long upload — and the case that
    // previously destroyed the whole recording.
    const { drive, provider } = setup({ networkErrorAttempts: [2] });
    const { fileId } = await provider.uploadResumable(makeFile(), { name: "a.webm", mimeType: "video/webm" });
    expect(fileId).toBe("file-complete");
    expect(drive.received).toBe(FILE_SIZE);
    // It re-probed rather than assuming where it was.
    expect(drive.probeCount).toBeGreaterThan(0);
  });

  it("retries a 503 and completes", async () => {
    const { drive, provider } = setup({ failChunkAttempts: [{ attempt: 2, status: 503 }] });
    const { fileId } = await provider.uploadResumable(makeFile(), { name: "a.webm", mimeType: "video/webm" });
    expect(fileId).toBe("file-complete");
    expect(drive.received).toBe(FILE_SIZE);
  });

  it("retries a 429 and completes", async () => {
    const { provider } = setup({ failChunkAttempts: [{ attempt: 1, status: 429 }] });
    const { fileId } = await provider.uploadResumable(makeFile(), { name: "a.webm", mimeType: "video/webm" });
    expect(fileId).toBe("file-complete");
  });

  it("honours the server's Range when it accepts less than we sent", async () => {
    // The old code assumed `offset = end` after a 308. A server that stores a partial chunk
    // would then leave a hole, and the FakeDrive rejects the wrong offset with a 400.
    const { drive, provider } = setup({ acceptOnlyBytes: 1024 * 1024 });
    const { fileId } = await provider.uploadResumable(makeFile(4 * 1024 * 1024), {
      name: "a.webm",
      mimeType: "video/webm",
    });
    expect(fileId).toBe("file-complete");
    expect(drive.received).toBe(4 * 1024 * 1024);
  });

  it("gives up with a retryable error after exhausting attempts", async () => {
    const drive = new FakeDrive({
      failChunkAttempts: Array.from({ length: 20 }, (_, i) => ({ attempt: i + 1, status: 500 })),
    });
    const provider = makeProvider(drive, { maxAttempts: 3 });
    await expect(
      provider.uploadResumable(makeFile(1024), { name: "a.webm", mimeType: "video/webm" }),
    ).rejects.toThrow(UploadError);
  });

  it("does not retry a non-transient failure", async () => {
    const drive = new FakeDrive({ failChunkAttempts: [{ attempt: 1, status: 403 }] });
    const provider = makeProvider(drive);
    await expect(
      provider.uploadResumable(makeFile(1024), { name: "a.webm", mimeType: "video/webm" }),
    ).rejects.toThrow(/403/);
    expect(drive.putAttempts).toBe(1);
  });

  it("reports an expired session distinctly, so the UI can restart rather than retry", async () => {
    const drive = new FakeDrive({ expireAfterAttempts: 0 });
    const provider = makeProvider(drive);
    await expect(
      provider.uploadResumable(makeFile(1024), { name: "a.webm", mimeType: "video/webm" }),
    ).rejects.toMatchObject({ sessionExpired: true });
  });
});

describe("resuming a persisted session", () => {
  it("probes the server for the real offset and continues from there", async () => {
    const drive = new FakeDrive();
    const provider = makeProvider(drive);

    // Simulate a page that already uploaded 8 MiB before being reloaded.
    drive.totalSize = FILE_SIZE;
    drive.received = 8 * 1024 * 1024;

    const { fileId } = await provider.uploadResumable(
      makeFile(),
      { name: "a.webm", mimeType: "video/webm" },
      { resumeSessionUri: drive.uri },
    );

    expect(fileId).toBe("file-complete");
    expect(drive.probeCount).toBeGreaterThan(0);
    // Crucially, it did NOT open a second session and start over.
    expect(drive.sessionsCreated).toBe(0);
  });

  it("returns immediately when the server already has the whole file", async () => {
    const drive = new FakeDrive();
    const provider = makeProvider(drive);
    drive.totalSize = FILE_SIZE;
    drive.received = FILE_SIZE;

    const { fileId } = await provider.uploadResumable(
      makeFile(),
      { name: "a.webm", mimeType: "video/webm" },
      { resumeSessionUri: drive.uri },
    );
    expect(fileId).toBe("file-complete");
    expect(drive.putAttempts).toBe(0);
  });

  it("starts a fresh session when the persisted one has expired", async () => {
    const drive = new FakeDrive();
    const provider = makeProvider(drive);
    const ended = vi.fn();

    // An unknown session URI answers 404 — Drive's signal that the session is gone. The
    // upload must recover by opening a new one, not fail the user's recording.
    const { fileId } = await provider.uploadResumable(
      makeFile(1024),
      { name: "a.webm", mimeType: "video/webm" },
      { resumeSessionUri: "https://upload.example/unknown-session", onSessionEnd: ended },
    );

    expect(fileId).toBe("file-complete");
    expect(drive.sessionsCreated).toBe(1);
    // The dead session is retired so the caller stops trying to resume it.
    expect(ended).toHaveBeenCalledWith("https://upload.example/unknown-session");
  });
});

describe("cancellation", () => {
  it("stops when the caller aborts", async () => {
    const { provider } = setup();
    const controller = new AbortController();
    controller.abort();
    await expect(
      provider.uploadResumable(
        makeFile(),
        { name: "a.webm", mimeType: "video/webm" },
        { signal: controller.signal },
      ),
    ).rejects.toThrow(/cancelled/i);
  });
});
