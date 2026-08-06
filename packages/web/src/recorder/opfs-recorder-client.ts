import type {
  DurationPatchOutcome,
  FinalizedLocation,
  OpfsWorkerRequest,
  OpfsWorkerResponse,
  RecordingContainer,
  StorageTier,
} from "./opfs-recorder.worker";

export interface OpfsRecorderDiagnostics {
  bytesWritten: number;
}

export interface OpfsFinalizeResult {
  bytesWritten: number;
  location: FinalizedLocation;
  /** Whether the WebM Info/Duration slot was filled, and why not when it wasn't. */
  duration: DurationPatchOutcome;
}

export interface OpfsRecorderHandle {
  /** Which storage tier the worker actually got — OPFS, or the IndexedDB fallback. */
  readonly tier: StorageTier;
  writeChunk: (data: ArrayBuffer) => void;
  /** `durationMs` fills the Duration slot reserved while the header streamed past. */
  finalize: (durationMs: number) => Promise<OpfsFinalizeResult>;
  getDiagnostics: () => OpfsRecorderDiagnostics;
  terminate: () => void;
}

export interface StartRecorderOptions {
  fileName: string;
  container: RecordingContainer;
  mimeType: string;
}

/** Starts the worker, waits for its "ready" ack, and returns a small typed client over it. */
export function startOpfsRecorder(options: StartRecorderOptions): Promise<OpfsRecorderHandle> {
  const worker = new Worker(new URL("./opfs-recorder.worker.ts", import.meta.url), { type: "module" });
  const diagnostics: OpfsRecorderDiagnostics = { bytesWritten: 0 };

  let finalizeResolve: ((result: OpfsFinalizeResult) => void) | null = null;
  let finalizeReject: ((err: Error) => void) | null = null;

  return new Promise<OpfsRecorderHandle>((resolveReady, rejectReady) => {
    const fail = (err: Error): void => {
      if (finalizeReject) {
        finalizeReject(err);
      } else {
        rejectReady(err);
      }
    };

    // A worker that dies mid-recording must not leave `finalize()` pending forever.
    worker.onerror = (event: ErrorEvent) => fail(new Error(event.message || "Recording worker crashed"));

    worker.onmessage = (event: MessageEvent<OpfsWorkerResponse>) => {
      const message = event.data;
      switch (message.type) {
        case "ready":
          resolveReady({
            tier: message.tier,
            writeChunk: (data) => {
              const request: OpfsWorkerRequest = { type: "chunk", data };
              worker.postMessage(request, [data]);
            },
            finalize: (durationMs) =>
              new Promise<OpfsFinalizeResult>((resolve, reject) => {
                finalizeResolve = resolve;
                finalizeReject = reject;
                const request: OpfsWorkerRequest = { type: "finalize", durationMs };
                worker.postMessage(request);
              }),
            getDiagnostics: () => ({ ...diagnostics }),
            terminate: () => worker.terminate(),
          });
          break;
        case "progress":
          diagnostics.bytesWritten = message.bytesWritten;
          break;
        case "finalized":
          finalizeResolve?.({
            bytesWritten: message.bytesWritten,
            location: message.location,
            duration: message.duration,
          });
          break;
        case "error":
          fail(new Error(message.message));
          break;
      }
    };

    const initRequest: OpfsWorkerRequest = {
      type: "init",
      fileName: options.fileName,
      container: options.container,
      mimeType: options.mimeType,
    };
    worker.postMessage(initRequest);
  });
}
