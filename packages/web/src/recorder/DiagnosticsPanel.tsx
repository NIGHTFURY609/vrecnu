import type { JSX } from "react";
import type { RecorderState } from "@vrecnu/core";
import { Button } from "@vrecnu/ui";
import type { SelectedContainer } from "./container-detection";

export type SeekCheckStatus = "idle" | "running" | "pass" | "fail";

export interface DiagnosticsPanelProps {
  state: RecorderState;
  container: SelectedContainer | null;
  compositedFrames: number;
  droppedVideoFrames: number;
  opfsBytesWritten: number;
  heapUsedBytes: number | null;
  durationSec: number | null;
  seekCheck: SeekCheckStatus;
  onRunSeekCheck: () => void;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
}

function StatRow({ label, value }: { label: string; value: string }): JSX.Element {
  return (
    <div className="flex justify-between border-b border-gray-100 py-1 text-sm">
      <span className="text-gray-500">{label}</span>
      <span className="font-mono">{value}</span>
    </div>
  );
}

/**
 * Surfaces the three numbers ADR-003's spike is proving: dropped frames (pacing), OPFS bytes
 * written vs. JS heap (flat-memory streaming), and a scripted seek check (valid duration).
 */
export function DiagnosticsPanel(props: DiagnosticsPanelProps): JSX.Element {
  const {
    state,
    container,
    compositedFrames,
    droppedVideoFrames,
    opfsBytesWritten,
    heapUsedBytes,
    durationSec,
    seekCheck,
    onRunSeekCheck,
  } = props;

  return (
    <div className="flex flex-col gap-1 rounded-md border border-gray-200 p-4">
      <StatRow label="Recorder state" value={state} />
      <StatRow label="Container" value={container ? `${container.container} (${container.mimeType})` : "—"} />
      <StatRow label="Composited frames" value={String(compositedFrames)} />
      <StatRow label="Dropped video frames" value={String(droppedVideoFrames)} />
      <StatRow label="OPFS bytes written" value={formatBytes(opfsBytesWritten)} />
      <StatRow label="JS heap used" value={heapUsedBytes !== null ? formatBytes(heapUsedBytes) : "unavailable"} />
      <StatRow label="Recorded duration" value={durationSec !== null ? `${durationSec.toFixed(1)}s` : "—"} />
      <div className="mt-2 flex items-center gap-3">
        <Button size="sm" variant="outline" onClick={onRunSeekCheck} disabled={durationSec === null}>
          Run seek check
        </Button>
        <span className="text-sm">
          {seekCheck === "idle" && "not run"}
          {seekCheck === "running" && "seeking…"}
          {seekCheck === "pass" && <span className="text-green-600">pass — finite duration, seek landed</span>}
          {seekCheck === "fail" && <span className="text-red-600">fail — duration or seek did not land</span>}
        </span>
      </div>
    </div>
  );
}
