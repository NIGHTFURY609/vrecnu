import { useState } from "react";
import { Button } from "@vrecnu/ui";
import type { AppError, AppState } from "../engine";
import { toAppError } from "../errors";
import { formatElapsed } from "../format";
import { stopRecording } from "../tauri";
import { ErrorNote } from "./ErrorNote";
import { DANGER } from "./styles";

/** Shown when the window is opened mid-recording; normally the window is closed while recording. */
export function Recording({ app }: { app: AppState }) {
  const [stopping, setStopping] = useState(false);
  const [error, setError] = useState<AppError | null>(null);
  const saving = app.session === "finalizing";

  const stop = () => {
    setStopping(true);
    setError(null);
    stopRecording().catch((err: unknown) => {
      setError(toAppError(err));
      setStopping(false);
    });
  };

  return (
    <section className="flex flex-col gap-4">
      <p className="text-4xl font-semibold tabular-nums">{formatElapsed(app.elapsedMs)}</p>
      <p className="text-sm text-slate-300">
        {saving ? "Saving the recording…" : "Recording. You can also stop it from the tray icon."}
      </p>
      {app.stats.framesDropped > 0 && (
        <p className="text-sm text-amber-300">{app.stats.framesDropped} frames dropped so far.</p>
      )}
      {error && <ErrorNote error={error} />}
      <Button variant="destructive" className={DANGER} disabled={stopping || saving} onClick={stop}>
        Stop
      </Button>
    </section>
  );
}
