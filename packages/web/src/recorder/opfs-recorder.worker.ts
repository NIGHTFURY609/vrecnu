/**
 * Dedicated Web Worker proving ADR-003's Amendment: chunks stream straight to disk instead of
 * piling up in a JS array. Runs in its own worker context — never import browser/DOM-only
 * code here that assumes `window`.
 *
 * Two storage tiers behind one {@link ChunkSink} interface (AGENTS.md §6):
 *   1. OPFS via `createSyncAccessHandle()` — the real thing. Random-access writes, no
 *      structured clone, no transactions.
 *   2. IndexedDB Blob chunks — the fallback where sync access handles are unavailable.
 *      Degraded but still disk-backed; previously this case just dead-ended the app.
 *
 * It also owns the WebM duration fix. MediaRecorder omits Info/Duration, and the obvious
 * remedy (`fix-webm-duration`) reads the whole recording into memory and hands back a new
 * Blob — undoing, in one line at finalize, the flat-memory guarantee the rest of this file
 * exists to provide. Instead the header is rewritten as it streams past (a few KB), leaving
 * an 8-byte slot that finalize overwrites in place. Memory stays flat; finalize is O(1).
 *
 * TypeScript's bundled DOM lib doesn't yet ship FileSystemSyncAccessHandle types, so that
 * surface is typed locally below rather than via a global augmentation (which would risk
 * colliding with whatever the installed TS version eventually adds).
 */

import { IdbChunkStore } from "./idb-chunk-store";
import { encodeDurationValue, planDurationPatch, type UnsupportedReason } from "./webm-duration";

interface SyncAccessHandle {
  // `ArrayBufferView | ArrayBufferLike` rather than `BufferSource`: since TS 5.7 a bare
  // `Uint8Array` is `Uint8Array<ArrayBufferLike>`, which `BufferSource` no longer accepts.
  // The real API takes shared buffers too, so this is the more faithful signature anyway.
  write(buffer: ArrayBufferView | ArrayBufferLike, options?: { at?: number }): number;
  flush(): void;
  close(): void;
  truncate(newSize: number): void;
  getSize(): number;
}

interface FileHandleWithSyncAccess extends FileSystemFileHandle {
  createSyncAccessHandle(): Promise<SyncAccessHandle>;
}

export type RecordingContainer = "webm" | "mp4";
export type StorageTier = "opfs" | "indexeddb";

export type OpfsWorkerRequest =
  | { type: "init"; fileName: string; container: RecordingContainer; mimeType: string }
  | { type: "chunk"; data: ArrayBuffer }
  | { type: "finalize"; durationMs: number };

export type DurationSkipReason = UnsupportedReason | "not-webm-container" | "header-not-found" | "no-duration";

export type DurationPatchOutcome = { applied: true } | { applied: false; reason: DurationSkipReason };

/** How the finished recording is retrieved — a path on OPFS, or an assembled Blob from IDB. */
export type FinalizedLocation = { tier: "opfs"; fileName: string } | { tier: "indexeddb"; blob: Blob };

export type OpfsWorkerResponse =
  | { type: "ready"; tier: StorageTier }
  | { type: "progress"; bytesWritten: number }
  | {
      type: "finalized";
      bytesWritten: number;
      location: FinalizedLocation;
      duration: DurationPatchOutcome;
    }
  | { type: "error"; message: string };

interface WorkerSelf {
  postMessage(message: OpfsWorkerResponse): void;
  onmessage: ((event: MessageEvent<OpfsWorkerRequest>) => void) | null;
}

const workerSelf = self as unknown as WorkerSelf;

/**
 * How much of the stream to buffer while looking for the end of Info. Chromium emits the
 * full header in the first chunk, so this is a safety valve, not a working budget — if it
 * is ever hit, we give up on patching rather than let the buffer become the accumulation
 * this worker exists to prevent.
 */
const HEADER_SCAN_LIMIT_BYTES = 1024 * 1024;

/** Storage-agnostic write surface. Sequential appends plus one random-access patch. */
interface ChunkSink {
  readonly tier: StorageTier;
  append(bytes: Uint8Array): Promise<number>;
  patch(bytes: Uint8Array, at: number): Promise<void>;
  finish(): Promise<FinalizedLocation>;
  dispose(): void;
}

class OpfsSink implements ChunkSink {
  readonly tier = "opfs" as const;
  private offset = 0;

  private constructor(
    private readonly handle: SyncAccessHandle,
    private readonly fileName: string,
  ) {}

  static async create(fileName: string): Promise<OpfsSink> {
    const root = await navigator.storage.getDirectory();
    const fileHandle = (await root.getFileHandle(fileName, { create: true })) as FileHandleWithSyncAccess;
    if (typeof fileHandle.createSyncAccessHandle !== "function") {
      throw new Error("createSyncAccessHandle is unavailable");
    }
    const handle = await fileHandle.createSyncAccessHandle();
    handle.truncate(0);
    return new OpfsSink(handle, fileName);
  }

  append(bytes: Uint8Array): Promise<number> {
    this.offset += this.handle.write(bytes, { at: this.offset });
    this.handle.flush();
    return Promise.resolve(this.offset);
  }

  patch(bytes: Uint8Array, at: number): Promise<void> {
    this.handle.write(bytes, { at });
    this.handle.flush();
    return Promise.resolve();
  }

  finish(): Promise<FinalizedLocation> {
    this.handle.close();
    return Promise.resolve({ tier: "opfs", fileName: this.fileName });
  }

  dispose(): void {
    try {
      this.handle.close();
    } catch {
      // Already closed by finish().
    }
  }
}

class IdbSink implements ChunkSink {
  readonly tier = "indexeddb" as const;

  private constructor(
    private readonly store: IdbChunkStore,
    private readonly mimeType: string,
  ) {}

  static async create(fileName: string, mimeType: string): Promise<IdbSink> {
    return new IdbSink(await IdbChunkStore.create(fileName), mimeType);
  }

  append(bytes: Uint8Array): Promise<number> {
    return this.store.append(bytes);
  }

  patch(bytes: Uint8Array, at: number): Promise<void> {
    return this.store.patch(bytes, at);
  }

  async finish(): Promise<FinalizedLocation> {
    const blob = await this.store.assemble(this.mimeType);
    this.store.close();
    return { tier: "indexeddb", blob };
  }

  dispose(): void {
    this.store.close();
  }
}

interface DurationSlot {
  readonly offset: number;
  readonly valueBytes: 4 | 8;
  readonly timecodeScaleNs: number;
}

let sink: ChunkSink | null = null;
let bytesWritten = 0;

/** False only while we are still buffering the WebM header looking for a Duration slot. */
let headerResolved = true;
let pendingHeader: Uint8Array[] = [];
let pendingHeaderBytes = 0;
let durationSlot: DurationSlot | null = null;
let durationSkipReason: DurationSkipReason | null = null;

/**
 * Serializes every sink operation. IndexedDB writes are async, and MediaRecorder chunks
 * arrive independently of them — without this queue, chunks could land out of order.
 */
let writeQueue: Promise<void> = Promise.resolve();

function post(message: OpfsWorkerResponse): void {
  workerSelf.postMessage(message);
}

function enqueue(work: () => Promise<void>): void {
  writeQueue = writeQueue.then(work).catch((err: unknown) => {
    post({ type: "error", message: err instanceof Error ? err.message : "Failed to write recording data" });
  });
}

function concatPending(): Uint8Array {
  const merged = new Uint8Array(pendingHeaderBytes);
  let at = 0;
  for (const part of pendingHeader) {
    merged.set(part, at);
    at += part.length;
  }
  return merged;
}

/** Flushes the buffered prefix verbatim and stops trying to patch. */
async function giveUpOnPatch(target: ChunkSink, reason: DurationSkipReason): Promise<void> {
  const buffered = concatPending();
  pendingHeader = [];
  pendingHeaderBytes = 0;
  headerResolved = true;
  durationSlot = null;
  durationSkipReason = reason;
  if (buffered.length > 0) {
    bytesWritten = await target.append(buffered);
  }
}

async function handleInit(fileName: string, container: RecordingContainer, mimeType: string): Promise<void> {
  let created: ChunkSink;
  try {
    created = await OpfsSink.create(fileName);
  } catch {
    // Safari before 17 and some WebViews have OPFS but not sync access handles. Falling back
    // keeps the recording rather than failing the session outright.
    created = await IdbSink.create(fileName, mimeType);
  }

  sink = created;
  bytesWritten = 0;
  pendingHeader = [];
  pendingHeaderBytes = 0;
  durationSlot = null;
  durationSkipReason = container === "webm" ? null : "not-webm-container";
  headerResolved = container !== "webm";
  writeQueue = Promise.resolve();
  post({ type: "ready", tier: created.tier });
}

function handleChunk(data: ArrayBuffer): void {
  const target = sink;
  if (!target) {
    post({ type: "error", message: "Received a chunk before init" });
    return;
  }
  const bytes = new Uint8Array(data);

  enqueue(async () => {
    if (headerResolved) {
      bytesWritten = await target.append(bytes);
      post({ type: "progress", bytesWritten });
      return;
    }

    pendingHeader.push(bytes);
    pendingHeaderBytes += bytes.length;
    const buffered = concatPending();
    const result = planDurationPatch(buffered);

    if (result.status === "incomplete") {
      if (pendingHeaderBytes > HEADER_SCAN_LIMIT_BYTES) {
        await giveUpOnPatch(target, "header-not-found");
        post({ type: "progress", bytesWritten });
      }
      return;
    }

    if (result.status === "unsupported") {
      await giveUpOnPatch(target, result.reason);
      post({ type: "progress", bytesWritten });
      return;
    }

    const { plan } = result;
    durationSlot = {
      offset: plan.valueOffset,
      valueBytes: plan.valueBytes,
      timecodeScaleNs: plan.timecodeScaleNs,
    };
    if (plan.kind === "in-place") {
      bytesWritten = await target.append(buffered);
    } else {
      await target.append(plan.header);
      bytesWritten = await target.append(buffered.subarray(plan.consumed));
    }

    pendingHeader = [];
    pendingHeaderBytes = 0;
    headerResolved = true;
    post({ type: "progress", bytesWritten });
  });
}

function handleFinalize(durationMs: number): void {
  const target = sink;
  if (!target) {
    post({ type: "error", message: "Finalize called before init" });
    return;
  }

  enqueue(async () => {
    // Anything still buffered never resolved into a header — write it through unchanged.
    if (!headerResolved) {
      await giveUpOnPatch(target, "header-not-found");
    }

    let duration: DurationPatchOutcome;
    if (durationSlot && Number.isFinite(durationMs) && durationMs > 0) {
      const value = encodeDurationValue(durationMs, durationSlot.timecodeScaleNs, durationSlot.valueBytes);
      await target.patch(value, durationSlot.offset);
      duration = { applied: true };
    } else if (durationSlot) {
      duration = { applied: false, reason: "no-duration" };
    } else {
      duration = { applied: false, reason: durationSkipReason ?? "header-not-found" };
    }

    const location = await target.finish();
    sink = null;
    post({ type: "finalized", bytesWritten, location, duration });
  });
}

workerSelf.onmessage = (event) => {
  const message = event.data;
  switch (message.type) {
    case "init":
      void handleInit(message.fileName, message.container, message.mimeType).catch((err: unknown) => {
        post({ type: "error", message: err instanceof Error ? err.message : "Recording storage init failed" });
      });
      break;
    case "chunk":
      handleChunk(message.data);
      break;
    case "finalize":
      handleFinalize(message.durationMs);
      break;
  }
};
