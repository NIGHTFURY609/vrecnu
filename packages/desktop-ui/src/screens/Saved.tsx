import { useState } from "react";
import { Button } from "@vrecnu/ui";
import type { AppError, AppState } from "../engine";
import { toAppError } from "../errors";
import { fileName, formatBytes, formatElapsed } from "../format";
import { resetSession, revealRecording } from "../tauri";
import { ErrorNote } from "./ErrorNote";
import { PRIMARY, SECONDARY } from "./styles";

/** S3's end of the line. S4 replaces it with Review (<video>) and Publish. */
export function Saved({ app }: { app: AppState }) {
  const [error, setError] = useState<AppError | null>(null);
  const run = (action: () => Promise<void>) => {
    setError(null);
    action().catch((err: unknown) => setError(toAppError(err)));
  };

  // Between the Stopped event and the snapshot that carries the file (see needsResync).
  if (app.recording === null) return <p className="text-sm text-slate-400">Saving the recording…</p>;
  const { recording } = app;

  return (
    <section className="flex flex-col gap-4">
      <h2 className="text-lg font-semibold">Saved</h2>
      <dl className="grid grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
        <dt className="text-slate-400">File</dt>
        <dd className="break-all">{fileName(recording.path)}</dd>
        <dt className="text-slate-400">Length</dt>
        <dd>{formatElapsed(recording.durationMs)}</dd>
        <dt className="text-slate-400">Size</dt>
        <dd>{formatBytes(recording.sizeBytes)}</dd>
      </dl>
      {app.stats.framesDropped > 0 && (
        <p className="text-sm text-amber-300">{app.stats.framesDropped} frames were dropped.</p>
      )}
      {error && <ErrorNote error={error} />}
      <div className="flex gap-2">
        <Button className={PRIMARY} onClick={() => run(revealRecording)}>
          Show in folder
        </Button>
        <Button variant="outline" className={SECONDARY} onClick={() => run(resetSession)}>
          New recording
        </Button>
      </div>
    </section>
  );
}
