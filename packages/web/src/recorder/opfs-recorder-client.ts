import type { OpfsWorkerRequest, OpfsWorkerResponse } from "./opfs-recorder.worker";

export interface OpfsRecorderDiagnostics {
  bytesWritten: number;
}

export interface OpfsFinalizeResult {
  bytesWritten: number;
  fileName: string;
}

export interface OpfsRecorderHandle {
  writeChunk: (data: ArrayBuffer) => void;
  finalize: () => Promise<OpfsFinalizeResult>;
  getDiagnostics: () => OpfsRecorderDiagnostics;
  terminate: () => void;
}

/** Starts the worker, waits for its "ready" ack, and returns a small typed client over it. */
export function startOpfsRecorder(fileName: string): Promise<OpfsRecorderHandle> {
  const worker = new Worker(new URL("./opfs-recorder.worker.ts", import.meta.url), { type: "module" });
  const diagnostics: OpfsRecorderDiagnostics = { bytesWritten: 0 };

  let finalizeResolve: ((result: OpfsFinalizeResult) => void) | null = null;
  let finalizeReject: ((err: Error) => void) | null = null;

  return new Promise<OpfsRecorderHandle>((resolveReady, rejectReady) => {
    worker.onmessage = (event: MessageEvent<OpfsWorkerResponse>) => {
      const message = event.data;
      switch (message.type) {
        case "ready":
          resolveReady({
            writeChunk: (data) => {
              const request: OpfsWorkerRequest = { type: "chunk", data };
              worker.postMessage(request, [data]);
            },
            finalize: () =>
              new Promise<OpfsFinalizeResult>((resolve, reject) => {
                finalizeResolve = resolve;
                finalizeReject = reject;
                const request: OpfsWorkerRequest = { type: "finalize" };
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
          finalizeResolve?.({ bytesWritten: message.bytesWritten, fileName: message.fileName });
          break;
        case "error":
          if (finalizeReject) {
            finalizeReject(new Error(message.message));
          } else {
            rejectReady(new Error(message.message));
          }
          break;
      }
    };

    const initRequest: OpfsWorkerRequest = { type: "init", fileName };
    worker.postMessage(initRequest);
  });
}
