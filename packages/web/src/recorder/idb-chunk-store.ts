/**
 * IndexedDB fallback tier for chunk streaming (AGENTS.md §6).
 *
 * OPFS sync access handles are the primary path and are strictly better: real random-access
 * writes, no structured-clone step, no transaction overhead. But `createSyncAccessHandle` is
 * absent in Safari's older releases and in some embedded/WebView contexts, and the previous
 * behaviour there was to post an error and dead-end the app — losing the recording outright.
 *
 * Chunks are stored as `Blob`s, never `ArrayBuffer`s. That distinction is the whole point:
 * browsers back Blobs with on-disk storage, so neither writing them nor concatenating them at
 * the end pulls the recording into RAM. Storing `ArrayBuffer`s here would reintroduce exactly
 * the accumulation ADR-003 exists to prevent, just one layer down.
 */

const DB_NAME = "vrecnu-recordings";
const DB_VERSION = 1;
const STORE = "chunks";

interface ChunkRecord {
  /** `${fileName}:${seq}` — IndexedDB needs a scalar key for `delete()` range scans to stay simple. */
  key: string;
  fileName: string;
  seq: number;
  /** Absolute byte offset of this chunk within the reconstructed file. */
  start: number;
  length: number;
  data: Blob;
}

/**
 * Copies into a freshly allocated `ArrayBuffer`. Two reasons, both load-bearing: since TS 5.7
 * a bare `Uint8Array` is `Uint8Array<ArrayBufferLike>`, which is not a `BlobPart`; and the
 * caller's buffer may be transferred or reused the moment this returns, so the Blob must not
 * alias it.
 */
function toOwnedBuffer(bytes: Uint8Array): ArrayBuffer {
  const copy = new ArrayBuffer(bytes.byteLength);
  new Uint8Array(copy).set(bytes);
  return copy;
}

function promisify<T>(request: IDBRequest<T>): Promise<T> {
  return new Promise((resolve, reject) => {
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error("IndexedDB request failed"));
  });
}

function openDb(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB_NAME, DB_VERSION);
    request.onupgradeneeded = () => {
      const db = request.result;
      if (!db.objectStoreNames.contains(STORE)) {
        const store = db.createObjectStore(STORE, { keyPath: "key" });
        store.createIndex("byFile", ["fileName", "seq"], { unique: true });
      }
    };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error ?? new Error("Failed to open IndexedDB"));
  });
}

export class IdbChunkStore {
  private seq = 0;
  private bytesWritten = 0;
  /** Kept so a patch can find the chunk covering a byte offset without a full scan. */
  private readonly boundaries: { seq: number; start: number; length: number }[] = [];

  private constructor(
    private readonly db: IDBDatabase,
    private readonly fileName: string,
  ) {}

  static async create(fileName: string): Promise<IdbChunkStore> {
    const db = await openDb();
    const store = new IdbChunkStore(db, fileName);
    await store.clear();
    return store;
  }

  get totalBytes(): number {
    return this.bytesWritten;
  }

  private tx(mode: IDBTransactionMode): IDBObjectStore {
    return this.db.transaction(STORE, mode).objectStore(STORE);
  }

  async append(bytes: Uint8Array): Promise<number> {
    const record: ChunkRecord = {
      key: `${this.fileName}:${this.seq}`,
      fileName: this.fileName,
      seq: this.seq,
      start: this.bytesWritten,
      length: bytes.length,
      data: new Blob([toOwnedBuffer(bytes)]),
    };
    await promisify(this.tx("readwrite").put(record));
    this.boundaries.push({ seq: this.seq, start: this.bytesWritten, length: bytes.length });
    this.seq++;
    this.bytesWritten += bytes.length;
    return this.bytesWritten;
  }

  /**
   * Overwrites `bytes` at absolute offset `at`. Only supports a patch contained within a
   * single chunk — which is all the duration fix needs, since its slot is inside the header
   * chunk. A cross-chunk patch throws rather than silently corrupting the file.
   */
  async patch(bytes: Uint8Array, at: number): Promise<void> {
    const target = this.boundaries.find(
      (chunk) => at >= chunk.start && at + bytes.length <= chunk.start + chunk.length,
    );
    if (!target) {
      throw new Error(`No single chunk covers bytes ${at}..${at + bytes.length}`);
    }
    const key = `${this.fileName}:${target.seq}`;
    const existing = await promisify(this.tx("readonly").get(key) as IDBRequest<ChunkRecord | undefined>);
    if (!existing) {
      throw new Error(`Chunk ${key} disappeared before it could be patched`);
    }
    const relative = at - target.start;
    const patched = new Blob([
      existing.data.slice(0, relative),
      new Blob([toOwnedBuffer(bytes)]),
      existing.data.slice(relative + bytes.length),
    ]);
    await promisify(this.tx("readwrite").put({ ...existing, data: patched }));
  }

  /** Concatenates the stored chunks into one Blob. Blob refs, so still disk-backed. */
  async assemble(mimeType: string): Promise<Blob> {
    const index = this.tx("readonly").index("byFile");
    const range = IDBKeyRange.bound([this.fileName, 0], [this.fileName, Number.MAX_SAFE_INTEGER]);
    const records = await promisify(index.getAll(range) as IDBRequest<ChunkRecord[]>);
    records.sort((a, b) => a.seq - b.seq);
    return new Blob(
      records.map((record) => record.data),
      { type: mimeType },
    );
  }

  /**
   * Cursor-based rather than getAllKeys-then-delete: an IndexedDB transaction closes as soon
   * as the microtask queue drains, so a store handle cannot survive an `await`.
   */
  clear(): Promise<void> {
    return new Promise((resolve, reject) => {
      const tx = this.db.transaction(STORE, "readwrite");
      const range = IDBKeyRange.bound([this.fileName, 0], [this.fileName, Number.MAX_SAFE_INTEGER]);
      const request = tx.objectStore(STORE).index("byFile").openCursor(range);
      request.onsuccess = () => {
        const cursor = request.result;
        if (cursor) {
          cursor.delete();
          cursor.continue();
        }
      };
      tx.oncomplete = () => resolve();
      tx.onerror = () => reject(tx.error ?? new Error("Failed to clear stored chunks"));
    });
  }

  close(): void {
    this.db.close();
  }
}

/** Deletes every stored chunk for every recording — used when reclaiming space. */
export async function clearAllIdbRecordings(): Promise<void> {
  try {
    const db = await openDb();
    await promisify(db.transaction(STORE, "readwrite").objectStore(STORE).clear());
    db.close();
  } catch {
    // Nothing to reclaim, or IndexedDB is unavailable. Not worth surfacing.
  }
}
