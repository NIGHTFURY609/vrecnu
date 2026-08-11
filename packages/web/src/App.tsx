import { useEffect, useRef, useState, type JSX } from "react";
import { Button } from "@vrecnu/ui";
import { useRecordingEngine } from "./recorder/useRecordingEngine";
import { SourcePicker, type RecordingSourceConfig } from "./recorder/SourcePicker";
import { DiagnosticsPanel, type SeekCheckStatus } from "./recorder/DiagnosticsPanel";
import { SessionRecovery } from "./recorder/SessionRecovery";
import type { RecordingSession } from "./recorder/opfs-sessions";
import { TrimPanel } from "./recorder/TrimPanel";
import { sanitizeFileName, saveRecordingToDisk, type SaveOutcome } from "./recorder/download";
import { CredentialsPanel, UploadPanel } from "@vrecnu/publish";
import { PwaPrompts } from "./pwa/PwaPrompts";

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
  const [recovered, setRecovered] = useState<{ file: File; session: RecordingSession } | null>(null);
  const [trimmedFile, setTrimmedFile] = useState<File | null>(null);
  const [nameBase, setNameBase] = useState("vrecnu-recording");
  const [saveState, setSaveState] = useState<SaveOutcome | "saving" | null>(null);
  const previewVideoRef = useRef<HTMLVideoElement>(null);
  const playbackVideoRef = useRef<HTMLVideoElement>(null);

  const isPicking = engine.state === "idle" || engine.state === "error";
  const isBusy = engine.state === "requesting-permissions" || engine.state === "ready";
  const isFinishing = engine.state === "stopping" || engine.state === "finalizing";
  const isRecording = engine.state === "recording";
  const stoppedFile = engine.state === "stopped" ? engine.resultFile : null;
  /** A just-finished recording and a recovered one are reviewed through the same surface. */
  const reviewFile = stoppedFile ?? recovered?.file ?? null;
  const reviewContainer = stoppedFile ? (engine.container?.container ?? "webm") : (recovered?.session.container ?? "webm");
  const reviewMimeType = stoppedFile
    ? (engine.container?.mimeType ?? "video/webm")
    : (recovered?.file.type ?? "video/webm");
  /** What upload and download actually operate on — the trimmed file when one exists. */
  const outputFile = trimmedFile ?? reviewFile;
  const outputFileName = `${sanitizeFileName(nameBase, "vrecnu-recording")}.${reviewContainer}`;

  useEffect(() => {
    const video = previewVideoRef.current;
    if (video) {
      video.srcObject = engine.previewStream;
    }
  }, [engine.previewStream]);

  useEffect(() => {
    const video = playbackVideoRef.current;
    if (!video || !outputFile) {
      return;
    }
    // Bound to the *output* file so applying a trim immediately updates what the user sees.
    const url = URL.createObjectURL(outputFile);
    video.src = url;
    return () => URL.revokeObjectURL(url);
  }, [outputFile]);

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

  /**
   * Leaving the review surface. A recovered file is only *dismissed*, never deleted — the
   * user may not have uploaded it yet, and discarding is an explicit choice offered in the
   * recovery panel. A freshly recorded one goes through `reset()`, which does reclaim it.
   */
  const handleDone = (): void => {
    setSeekCheck("idle");
    setTrimmedFile(null);
    setSaveState(null);
    setNameBase("vrecnu-recording");
    if (recovered) {
      setRecovered(null);
      return;
    }
    engine.reset();
  };

  const handleSaveToDisk = async (): Promise<void> => {
    if (!outputFile) {
      return;
    }
    setSaveState("saving");
    setSaveState(await saveRecordingToDisk(outputFile, outputFileName, reviewMimeType));
  };

  const handleRunSeekCheck = async (): Promise<void> => {
    if (!reviewFile) {
      return;
    }
    setSeekCheck("running");
    try {
      const passed = await runSeekCheck(reviewFile);
      setSeekCheck(passed ? "pass" : "fail");
    } catch {
      setSeekCheck("fail");
    }
  };

  return (
    <main className="mx-auto flex min-h-screen max-w-2xl flex-col items-center gap-6 p-8">
      <h1 className="text-2xl font-semibold">vrecnu</h1>

      <PwaPrompts />

      {isPicking && !reviewFile ? (
        <div className="flex w-full flex-col gap-4">
          <SessionRecovery
            onRecover={(file, session) => {
              setSeekCheck("idle");
              setRecovered({ file, session });
            }}
          />
          <SourcePicker value={sourceConfig} onChange={setSourceConfig} />
          <CredentialsPanel buildTimeClientId={import.meta.env.VITE_GOOGLE_CLIENT_ID} />
          {engine.errorMessage ? (
            <p className="text-sm text-red-600" role="alert">
              {engine.errorMessage}
            </p>
          ) : null}
          <Button onClick={() => void handleStart()} disabled={!sourceConfig.screen && !sourceConfig.webcam}>
            Start recording
          </Button>
        </div>
      ) : null}

      <div aria-live="polite" className="w-full text-center">
        {isBusy ? <p className="text-sm text-gray-500">Requesting access…</p> : null}
        {isFinishing ? <p className="text-sm text-gray-500">Finishing up…</p> : null}
      </div>

      {isRecording ? (
        <div className="flex w-full flex-col items-center gap-4">
          <video ref={previewVideoRef} autoPlay muted playsInline className="aspect-video w-full rounded-md bg-black" />
          <div className="flex items-center gap-4">
            <span className="font-mono text-lg" aria-hidden="true">
              {formatElapsed(elapsedSec)}
            </span>
            <span className="sr-only" role="status">
              Recording, {elapsedSec} seconds elapsed
            </span>
            <Button variant="destructive" onClick={engine.stop}>
              Stop
            </Button>
          </div>
        </div>
      ) : null}

      {reviewFile ? (
        <div className="flex w-full flex-col gap-4">
          {recovered ? (
            <p className="text-sm text-amber-800" role="status">
              Recovered from {new Date(recovered.session.startedAt).toLocaleString()}.
            </p>
          ) : null}
          <video ref={playbackVideoRef} controls className="aspect-video w-full rounded-md bg-black" />

          <TrimPanel
            file={reviewFile}
            mimeType={reviewMimeType}
            container={reviewContainer}
            onSeek={(seconds) => {
              if (playbackVideoRef.current) {
                playbackVideoRef.current.currentTime = seconds;
              }
            }}
            onTrimmed={setTrimmedFile}
          />

          <label className="flex flex-col gap-1 text-sm">
            <span className="text-gray-700">File name</span>
            <span className="flex items-center gap-2">
              <input
                type="text"
                value={nameBase}
                onChange={(event) => setNameBase(event.target.value)}
                className="flex-1 rounded-md border border-slate-300 px-2 py-1"
              />
              <span className="text-gray-500">.{reviewContainer}</span>
            </span>
          </label>

          <div className="flex flex-wrap items-center gap-2">
            <Button variant="outline" onClick={() => void handleSaveToDisk()} disabled={saveState === "saving"}>
              {saveState === "saving" ? "Saving…" : "Save a local copy"}
            </Button>
            {saveState === "saved" ? (
              <span className="text-sm text-green-700" role="status">
                Saved.
              </span>
            ) : null}
            {saveState === "unsupported-fallback" ? (
              <span className="text-sm text-gray-600" role="status">
                Downloaded to your browser&apos;s downloads folder.
              </span>
            ) : null}
          </div>

          {outputFile ? (
            <UploadPanel
              source={outputFile}
              fileName={outputFileName}
              mimeType={reviewMimeType}
              durationWarning={
                engine.durationOutcome &&
                !engine.durationOutcome.applied &&
                engine.durationOutcome.reason !== "not-webm-container"
                  ? "This recording has no duration recorded, so it may not seek correctly in Drive's preview player."
                  : null
              }
            />
          ) : null}

          <Button variant="outline" onClick={handleDone}>
            {recovered ? "Done" : "Record again"}
          </Button>
        </div>
      ) : null}

      <div className="w-full">
        <button
          type="button"
          className="text-sm text-gray-500 underline"
          aria-expanded={showDiagnostics}
          aria-controls="vrecnu-diagnostics"
          onClick={() => setShowDiagnostics((prev) => !prev)}
        >
          {showDiagnostics ? "Hide diagnostics" : "Show diagnostics"}
        </button>
        {showDiagnostics ? (
          <div className="mt-2" id="vrecnu-diagnostics">
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
