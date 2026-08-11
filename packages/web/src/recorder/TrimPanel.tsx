import { useCallback, useEffect, useRef, useState, type JSX } from "react";
import { Button } from "@vrecnu/ui";
import { buildTrimmedWebm, indexWebmClusters, type IndexFailure, type WebmIndex } from "./webm-trim";

export interface TrimPanelProps {
  file: File;
  mimeType: string;
  container: string;
  /** Seeks the review player, so dragging a handle scrubs the video the user is watching. */
  onSeek?: (seconds: number) => void;
  /** Called with the trimmed file, or with `null` when the user clears the trim. */
  onTrimmed: (file: File | null) => void;
}

const NUDGE_MS = 100;
const COARSE_NUDGE_MS = 1_000;

function formatTimecode(ms: number): string {
  const total = Math.max(0, Math.floor(ms / 1000));
  const minutes = Math.floor(total / 60);
  const seconds = total % 60;
  const tenths = Math.floor((Math.max(0, ms) % 1000) / 100);
  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}.${tenths}`;
}

function describeFailure(reason: IndexFailure): string {
  switch (reason) {
    case "not-webm":
      return "Trimming is only available for WebM recordings.";
    case "unknown-cluster-size":
      return "This recording's internal layout doesn't support a lossless trim.";
    case "no-clusters":
      return "This recording has no video data to trim.";
    case "malformed":
      return "This recording's structure could not be read for trimming.";
  }
}

/**
 * Trim by Cluster boundary — see `webm-trim.ts` for why that, and not a byte-range cut or a
 * re-encode. Cut points snap to keyframes, so the panel reports back where the cut actually
 * landed rather than pretending it was frame-exact.
 */
export function TrimPanel({ file, mimeType, container, onSeek, onTrimmed }: TrimPanelProps): JSX.Element | null {
  const [index, setIndex] = useState<WebmIndex | null>(null);
  const [failure, setFailure] = useState<IndexFailure | null>(null);
  const [startMs, setStartMs] = useState(0);
  const [endMs, setEndMs] = useState(0);
  const [applied, setApplied] = useState<{ startMs: number; endMs: number; sizeBytes: number } | null>(null);
  const [busy, setBusy] = useState(false);
  const durationRef = useRef(0);

  // Derived, not stored: an unsupported container is knowable at render time, so putting it
  // in state would mean a synchronous setState inside the effect and a cascading render.
  const unsupportedContainer = container !== "webm";

  useEffect(() => {
    if (unsupportedContainer) {
      return;
    }
    let cancelled = false;

    void (async () => {
      const result = await indexWebmClusters(file);
      if (cancelled) {
        return;
      }
      setApplied(null);
      if (!result.ok) {
        setIndex(null);
        setFailure(result.reason);
        return;
      }
      setFailure(null);
      setIndex(result.index);
      durationRef.current = result.index.durationMs;
      setStartMs(0);
      setEndMs(result.index.durationMs);
    })();

    return () => {
      cancelled = true;
    };
  }, [file, unsupportedContainer]);

  const nudge = useCallback(
    (which: "start" | "end", deltaMs: number) => {
      if (which === "start") {
        setStartMs((prev) => Math.max(0, Math.min(prev + deltaMs, endMs - NUDGE_MS)));
      } else {
        setEndMs((prev) => Math.min(durationRef.current, Math.max(prev + deltaMs, startMs + NUDGE_MS)));
      }
    },
    [startMs, endMs],
  );

  if (unsupportedContainer || failure) {
    return (
      <p className="text-sm text-gray-500" role="status">
        {describeFailure(unsupportedContainer ? "not-webm" : failure!)}
      </p>
    );
  }

  if (!index) {
    return (
      <p className="text-sm text-gray-500" role="status">
        Preparing trim…
      </p>
    );
  }

  const duration = index.durationMs;
  const isTrimmed = startMs > 0 || endMs < duration;

  const handleApply = async (): Promise<void> => {
    setBusy(true);
    try {
      const result = await buildTrimmedWebm(file, index, { startMs, endMs }, mimeType);
      if (!result) {
        return;
      }
      onTrimmed(new File([result.blob], file.name, { type: mimeType }));
      setApplied({ startMs: result.actualStartMs, endMs: result.actualEndMs, sizeBytes: result.blob.size });
    } finally {
      setBusy(false);
    }
  };

  const handleReset = (): void => {
    setStartMs(0);
    setEndMs(duration);
    setApplied(null);
    onTrimmed(null);
  };

  return (
    <section className="flex flex-col gap-3 rounded-md border border-slate-200 p-3">
      <div className="flex items-baseline justify-between">
        <h2 className="text-sm font-medium">Trim</h2>
        <span className="font-mono text-xs text-gray-600">
          {formatTimecode(startMs)} – {formatTimecode(endMs)} ({formatTimecode(endMs - startMs)})
        </span>
      </div>

      <label className="flex flex-col gap-1 text-xs text-gray-600">
        Start
        <input
          type="range"
          min={0}
          max={duration}
          step={NUDGE_MS}
          value={startMs}
          onChange={(event) => {
            const next = Math.min(Number(event.target.value), endMs - NUDGE_MS);
            setStartMs(Math.max(0, next));
            onSeek?.(Math.max(0, next) / 1000);
          }}
          aria-label="Trim start"
          aria-valuetext={formatTimecode(startMs)}
        />
      </label>

      <label className="flex flex-col gap-1 text-xs text-gray-600">
        End
        <input
          type="range"
          min={0}
          max={duration}
          step={NUDGE_MS}
          value={endMs}
          onChange={(event) => {
            const next = Math.max(Number(event.target.value), startMs + NUDGE_MS);
            setEndMs(Math.min(duration, next));
            onSeek?.(Math.min(duration, next) / 1000);
          }}
          aria-label="Trim end"
          aria-valuetext={formatTimecode(endMs)}
        />
      </label>

      <div className="flex flex-wrap gap-2">
        <Button variant="outline" size="sm" onClick={() => nudge("start", -COARSE_NUDGE_MS)}>
          Start −1s
        </Button>
        <Button variant="outline" size="sm" onClick={() => nudge("start", COARSE_NUDGE_MS)}>
          Start +1s
        </Button>
        <Button variant="outline" size="sm" onClick={() => nudge("end", -COARSE_NUDGE_MS)}>
          End −1s
        </Button>
        <Button variant="outline" size="sm" onClick={() => nudge("end", COARSE_NUDGE_MS)}>
          End +1s
        </Button>
      </div>

      <div className="flex flex-wrap items-center gap-2">
        <Button size="sm" disabled={!isTrimmed || busy} onClick={() => void handleApply()}>
          {busy ? "Trimming…" : "Apply trim"}
        </Button>
        {applied ? (
          <Button variant="outline" size="sm" onClick={handleReset}>
            Undo trim
          </Button>
        ) : null}
      </div>

      {applied ? (
        <p className="text-xs text-green-700" role="status">
          Trimmed to {formatTimecode(applied.startMs)} – {formatTimecode(applied.endMs)}. Cuts snap to the nearest
          keyframe, so the result may differ slightly from the handles.
        </p>
      ) : null}
    </section>
  );
}
