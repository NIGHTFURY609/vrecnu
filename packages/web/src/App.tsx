import { useEffect, useRef, useState, type JSX } from "react";
import { Button } from "@vrecnu/ui";
import { useRecordingEngine } from "./recorder/useRecordingEngine";
import { SourcePicker, type RecordingSourceConfig } from "./recorder/SourcePicker";
import { DiagnosticsPanel, type SeekCheckStatus } from "./recorder/DiagnosticsPanel";
import { UploadPanel } from "./upload/UploadPanel";

const DEFAULT_SOURCE_CONFIG: RecordingSourceConfig = { screen: true, webcam: true, mic: true };

function formatElapsed(totalSeconds: number): string {
  const minutes = Math.floor(totalSeconds / 60);
  const seconds = totalSeconds % 60;
  return `${String(minutes).padStart(2, "0")}:${String(seconds).padStart(2, "0")}`;
}

/** Proves ADR-003's "valid, seekable output" risk on the actual recorded file, not a sample. */
async function runSeekCheck(blob: Blob): Promise<boolean> {
  const url = URL.createObjectURL(blob);
  const video = document.createElement("video");
  try {
    video.src = url;
    await new Promise<void>((resolve, reject) => {
      video.addEventListener("loadedmetadata", () => resolve(), { once: true });
      video.addEventListener("error", () => reject(new Error("Failed to load recording")), { once: true });
    });
    if (!Number.isFinite(video.duration) || video.duration <= 0) {
      return false;
    }
    await new Promise<void>((resolve, reject) => {
      video.addEventListener("seeked", () => resolve(), { once: true });
      video.addEventListener("error", () => reject(new Error("Seek failed")), { once: true });
      video.currentTime = video.duration / 2;
    });
    return true;
  } finally {
    URL.revokeObjectURL(url);
  }
}

export default function App(): JSX.Element {
  const engine = useRecordingEngine();
  const [sourceConfig, setSourceConfig] = useState<RecordingSourceConfig>(DEFAULT_SOURCE_CONFIG);
  const [elapsedSec, setElapsedSec] = useState(0);
  const [showDiagnostics, setShowDiagnostics] = useState(false);
  const [seekCheck, setSeekCheck] = useState<SeekCheckStatus>("idle");
  const previewVideoRef = useRef<HTMLVideoElement>(null);
  const playbackVideoRef = useRef<HTMLVideoElement>(null);

  const isPicking = engine.state === "idle" || engine.state === "error";
  const isBusy = engine.state === "requesting-permissions" || engine.state === "ready";
  const isFinishing = engine.state === "stopping" || engine.state === "finalizing";
  const isRecording = engine.state === "recording";
  const stoppedBlob = engine.state === "stopped" ? engine.resultBlob : null;

  useEffect(() => {
    const video = previewVideoRef.current;
    if (video) {
      video.srcObject = engine.previewStream;
    }
  }, [engine.previewStream]);

  useEffect(() => {
    const video = playbackVideoRef.current;
    if (!video || !stoppedBlob) {
      return;
    }
    const url = URL.createObjectURL(stoppedBlob);
    video.src = url;
    return () => URL.revokeObjectURL(url);
  }, [stoppedBlob]);

  useEffect(() => {
    if (!isRecording) {
      return;
    }
    const startedAt = Date.now();
    const interval = window.setInterval(() => setElapsedSec(Math.floor((Date.now() - startedAt) / 1000)), 250);
    return () => window.clearInterval(interval);
  }, [isRecording]);

  const handleStart = async (): Promise<void> => {
    setSeekCheck("idle");
    try {
      await engine.start(sourceConfig);
    } catch {
      // engine.errorMessage already carries the reason — surfaced in the picker view below.
    }
  };

  const handleRunSeekCheck = async (): Promise<void> => {
    if (!stoppedBlob) {
      return;
    }
    setSeekCheck("running");
    try {
      const passed = await runSeekCheck(stoppedBlob);
      setSeekCheck(passed ? "pass" : "fail");
    } catch {
      setSeekCheck("fail");
    }
  };

  return (
    <main className="mx-auto flex min-h-screen max-w-2xl flex-col items-center gap-6 p-8">
      <h1 className="text-2xl font-semibold">vrecnu</h1>

      {isPicking ? (
        <div className="flex w-full flex-col gap-4">
          <SourcePicker value={sourceConfig} onChange={setSourceConfig} />
          {engine.errorMessage ? <p className="text-sm text-red-600">{engine.errorMessage}</p> : null}
          <Button onClick={() => void handleStart()} disabled={!sourceConfig.screen && !sourceConfig.webcam}>
            Start recording
          </Button>
        </div>
      ) : null}

      {isBusy ? <p className="text-sm text-gray-500">Requesting access…</p> : null}
      {isFinishing ? <p className="text-sm text-gray-500">Finishing up…</p> : null}

      {isRecording ? (
        <div className="flex w-full flex-col items-center gap-4">
          <video ref={previewVideoRef} autoPlay muted playsInline className="aspect-video w-full rounded-md bg-black" />
          <div className="flex items-center gap-4">
            <span className="font-mono text-lg">{formatElapsed(elapsedSec)}</span>
            <Button variant="destructive" onClick={engine.stop}>
              Stop
            </Button>
          </div>
        </div>
      ) : null}

      {stoppedBlob ? (
        <div className="flex w-full flex-col gap-4">
          <video ref={playbackVideoRef} controls className="aspect-video w-full rounded-md bg-black" />
          <UploadPanel
            file={stoppedBlob}
            fileName={`vrecnu-recording.${engine.container?.container ?? "webm"}`}
            mimeType={engine.container?.mimeType ?? "video/webm"}
          />
          <Button variant="outline" onClick={engine.reset}>
            Record again
          </Button>
        </div>
      ) : null}

      <div className="w-full">
        <button
          type="button"
          className="text-sm text-gray-500 underline"
          onClick={() => setShowDiagnostics((prev) => !prev)}
        >
          {showDiagnostics ? "Hide diagnostics" : "Show diagnostics"}
        </button>
        {showDiagnostics ? (
          <div className="mt-2">
            <DiagnosticsPanel
              state={engine.state}
              container={engine.container}
              compositedFrames={engine.diagnostics.compositedFrames}
              droppedVideoFrames={engine.diagnostics.droppedVideoFrames}
              opfsBytesWritten={engine.diagnostics.opfsBytesWritten}
              heapUsedBytes={engine.diagnostics.heapUsedBytes}
              durationSec={engine.diagnostics.durationSec}
              seekCheck={seekCheck}
              onRunSeekCheck={() => void handleRunSeekCheck()}
            />
          </div>
        ) : null}
      </div>
    </main>
  );
}
