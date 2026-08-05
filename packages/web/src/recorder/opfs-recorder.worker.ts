/**
 * Dedicated Web Worker proving ADR-003's Amendment: chunks stream straight to OPFS via a
 * sync access handle (worker-only API, flat memory, no in-tab accumulation) instead of
 * piling up in a JS array. Runs in its own worker context — never import browser/DOM-only
 * code here that assumes `window`.
 *
 * TypeScript's bundled DOM lib doesn't yet ship FileSystemSyncAccessHandle types, so that
 * surface is typed locally below rather than via a global augmentation (which would risk
 * colliding with whatever the installed TS version eventually adds).
 */

interface SyncAccessHandle {
  write(buffer: BufferSource, options?: { at?: number }): number;
  flush(): void;
  close(): void;
  truncate(newSize: number): void;
  getSize(): number;
}

interface FileHandleWithSyncAccess extends FileSystemFileHandle {
  createSyncAccessHandle(): Promise<SyncAccessHandle>;
}

export type OpfsWorkerRequest =
  | { type: "init"; fileName: string }
  | { type: "chunk"; data: ArrayBuffer }
  | { type: "finalize" };

export type OpfsWorkerResponse =
  | { type: "ready" }
  | { type: "progress"; bytesWritten: number }
  | { type: "finalized"; bytesWritten: number; fileName: string }
  | { type: "error"; message: string };

interface WorkerSelf {
  postMessage(message: OpfsWorkerResponse): void;
  onmessage: ((event: MessageEvent<OpfsWorkerRequest>) => void) | null;
}

const workerSelf = self as unknown as WorkerSelf;

let accessHandle: SyncAccessHandle | null = null;
let offset = 0;
let currentFileName: string | null = null;

function post(message: OpfsWorkerResponse): void {
  workerSelf.postMessage(message);
}

async function handleInit(fileName: string): Promise<void> {
  const root = await navigator.storage.getDirectory();
  const fileHandle = (await root.getFileHandle(fileName, { create: true })) as FileHandleWithSyncAccess;
  const handle = await fileHandle.createSyncAccessHandle();
  handle.truncate(0);
  accessHandle = handle;
  offset = 0;
  currentFileName = fileName;
  post({ type: "ready" });
}

function handleChunk(data: ArrayBuffer): void {
  if (!accessHandle) {
    post({ type: "error", message: "Received a chunk before init" });
    return;
  }
  const written = accessHandle.write(data, { at: offset });
  offset += written;
  accessHandle.flush();
  post({ type: "progress", bytesWritten: offset });
}

function handleFinalize(): void {
  if (!accessHandle || currentFileName === null) {
    post({ type: "error", message: "Finalize called before init" });
    return;
  }
  accessHandle.close();
  const bytesWritten = offset;
  const fileName = currentFileName;
  accessHandle = null;
  currentFileName = null;
  post({ type: "finalized", bytesWritten, fileName });
}

workerSelf.onmessage = (event) => {
  const message = event.data;
  switch (message.type) {
    case "init":
      void handleInit(message.fileName).catch((err: unknown) => {
        post({ type: "error", message: err instanceof Error ? err.message : "OPFS init failed" });
      });
      break;
    case "chunk":
      handleChunk(message.data);
      break;
    case "finalize":
      handleFinalize();
      break;
  }
};
