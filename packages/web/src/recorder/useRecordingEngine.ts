import { useCallback, useEffect, useRef, useState } from "react";
import {
  IllegalRecorderTransitionError,
  INITIAL_RECORDER_STATE,
  recorderReducer,
  type RecorderEvent,
  type RecorderState,
} from "@vrecnu/core";
import { startComposite, type CompositeHandle } from "./composite";
import { selectContainer, type SelectedContainer } from "./container-detection";
import { startOpfsRecorder, type OpfsRecorderHandle } from "./opfs-recorder-client";
import { patchWebmDuration } from "./webm-duration-fix";
import type { RecordingSourceConfig } from "./SourcePicker";

const RECORDER_TIMESLICE_MS = 2000;
const DIAGNOSTICS_POLL_MS = 1000;

export interface RecordingDiagnostics {
  compositedFrames: number;
  droppedVideoFrames: number;
  opfsBytesWritten: number;
  heapUsedBytes: number | null;
  durationSec: number | null;
}

const INITIAL_DIAGNOSTICS: RecordingDiagnostics = {
  compositedFrames: 0,
  droppedVideoFrames: 0,
  opfsBytesWritten: 0,
  heapUsedBytes: null,
  durationSec: null,
};

export interface UseRecordingEngineResult {
  state: RecorderState;
  container: SelectedContainer | null;
  diagnostics: RecordingDiagnostics;
  resultBlob: Blob | null;
  errorMessage: string | null;
  previewStream: MediaStream | null;
  start: (config: RecordingSourceConfig) => Promise<void>;
  stop: () => void;
  reset: () => void;
}

interface PerformanceMemory {
  usedJSHeapSize: number;
}

function readHeapUsedBytes(): number | null {
  const memory = (performance as Performance & { memory?: PerformanceMemory }).memory;
  return memory ? memory.usedJSHeapSize : null;
}

/**
 * MediaRecorder does not mix multiple audio tracks placed on the same MediaStream — only
 * one is reliably recorded. Screen and mic audio are merged into a single track via a
 * MediaStreamAudioDestinationNode instead.
 */
function mergeAudioTracks(audioContext: AudioContext, streams: MediaStream[]): MediaStreamTrack | null {
  const streamsWithAudio = streams.filter((stream) => stream.getAudioTracks().length > 0);
  if (streamsWithAudio.length === 0) {
    return null;
  }
  if (streamsWithAudio.length === 1) {
    return streamsWithAudio[0]?.getAudioTracks()[0] ?? null;
  }
  const destination = audioContext.createMediaStreamDestination();
  for (const stream of streamsWithAudio) {
    audioContext.createMediaStreamSource(stream).connect(destination);
  }
  return destination.stream.getAudioTracks()[0] ?? null;
}

function createOffscreenVideo(stream: MediaStream): HTMLVideoElement {
  const video = document.createElement("video");
  video.srcObject = stream;
  video.muted = true;
  video.playsInline = true;
  return video;
}

async function waitForVideoReady(video: HTMLVideoElement): Promise<void> {
  if (video.readyState < HTMLMediaElement.HAVE_METADATA) {
    await new Promise<void>((resolve) => {
      video.addEventListener("loadedmetadata", () => resolve(), { once: true });
    });
  }
  await video.play();
}

/**
 * Generalizes the recorder spike's orchestration (proven in Slice 1: requestVideoFrameCallback
 * frame pacing, OPFS flat-memory streaming, valid/seekable output duration, screen+mic audio
 * merge) into a reusable hook driven by a SourcePicker selection instead of a hardcoded
 * screen+webcam+mic set. Screen-off recordings skip the canvas composite entirely and record
 * the webcam track directly — there is nothing to overlay.
 */
export function useRecordingEngine(): UseRecordingEngineResult {
  const [state, setState] = useState<RecorderState>(INITIAL_RECORDER_STATE);
  const [container, setContainer] = useState<SelectedContainer | null>(null);
  const [diagnostics, setDiagnostics] = useState<RecordingDiagnostics>(INITIAL_DIAGNOSTICS);
  const [resultBlob, setResultBlob] = useState<Blob | null>(null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [previewStream, setPreviewStream] = useState<MediaStream | null>(null);

  const screenStreamRef = useRef<MediaStream | null>(null);
  const webcamStreamRef = useRef<MediaStream | null>(null);
  const micStreamRef = useRef<MediaStream | null>(null);
  const compositeRef = useRef<CompositeHandle | null>(null);
  const opfsRef = useRef<OpfsRecorderHandle | null>(null);
  const mediaRecorderRef = useRef<MediaRecorder | null>(null);
  const startTimeRef = useRef(0);
  const pendingWritesRef = useRef<Promise<void>>(Promise.resolve());
  const pollRef = useRef<number | null>(null);
  const audioContextRef = useRef<AudioContext | null>(null);

  const send = useCallback((event: RecorderEvent) => {
    setState((prev) => {
      try {
        return recorderReducer(prev, event);
      } catch (err) {
        if (err instanceof IllegalRecorderTransitionError) {
          return prev;
        }
        throw err;
      }
    });
  }, []);

  const stopAllTracks = useCallback((): void => {
    screenStreamRef.current?.getTracks().forEach((track) => track.stop());
    webcamStreamRef.current?.getTracks().forEach((track) => track.stop());
    micStreamRef.current?.getTracks().forEach((track) => track.stop());
    screenStreamRef.current = null;
    webcamStreamRef.current = null;
    micStreamRef.current = null;
    void audioContextRef.current?.close();
    audioContextRef.current = null;
  }, []);

  useEffect(
    () => () => {
      compositeRef.current?.stop();
      opfsRef.current?.terminate();
      stopAllTracks();
      if (pollRef.current !== null) {
        window.clearInterval(pollRef.current);
      }
    },
    [stopAllTracks],
  );

  const start = useCallback(
    async (config: RecordingSourceConfig): Promise<void> => {
      if (!config.screen && !config.webcam) {
        throw new Error("Select at least Screen or Webcam to record.");
      }

      setErrorMessage(null);
      setResultBlob(null);
      setDiagnostics(INITIAL_DIAGNOSTICS);

      let selected: SelectedContainer;
      try {
        selected = selectContainer();
      } catch (err) {
        const message = err instanceof Error ? err.message : "No supported recording format found";
        setErrorMessage(message);
        send({ type: "ERROR", reason: message });
        throw err;
      }
      setContainer(selected);

      send({ type: "REQUEST_PERMISSIONS" });

      let screenStream: MediaStream | null = null;
      let webcamStream: MediaStream | null = null;
      let micStream: MediaStream | null = null;

      try {
        if (config.screen) {
          screenStream = await navigator.mediaDevices.getDisplayMedia({ video: true, audio: true });
          screenStreamRef.current = screenStream;
        }
        if (config.webcam) {
          webcamStream = await navigator.mediaDevices.getUserMedia({
            video: config.cameraDeviceId ? { deviceId: { exact: config.cameraDeviceId } } : true,
          });
          webcamStreamRef.current = webcamStream;
        }
        if (config.mic) {
          micStream = await navigator.mediaDevices.getUserMedia({
            audio: config.micDeviceId ? { deviceId: { exact: config.micDeviceId } } : true,
          });
          micStreamRef.current = micStream;
        }
      } catch (err) {
        const message = err instanceof Error ? err.message : "Failed to acquire recording sources";
        setErrorMessage(message);
        send({ type: "ERROR", reason: message });
        stopAllTracks();
        throw err;
      }

      send({ type: "PERMISSIONS_GRANTED" });

      let videoTrack: MediaStreamTrack;
      let liveStream: MediaStream;

      if (config.screen && screenStream) {
        const screenVideo = createOffscreenVideo(screenStream);
        await waitForVideoReady(screenVideo);

        let webcamVideo: HTMLVideoElement | null = null;
        if (webcamStream) {
          webcamVideo = createOffscreenVideo(webcamStream);
          await waitForVideoReady(webcamVideo);
        }

        const composite = startComposite({
          screenVideo,
          webcamVideo,
          width: screenVideo.videoWidth,
          height: screenVideo.videoHeight,
          onFrame: (frameDiagnostics) =>
            setDiagnostics((prev) => ({
              ...prev,
              compositedFrames: frameDiagnostics.compositedFrames,
              droppedVideoFrames: frameDiagnostics.droppedVideoFrames,
            })),
        });
        compositeRef.current = composite;

        const [track] = composite.stream.getVideoTracks();
        if (!track) {
          throw new Error("Composite canvas produced no video track");
        }
        videoTrack = track;
        liveStream = composite.stream;
      } else if (webcamStream) {
        const [track] = webcamStream.getVideoTracks();
        if (!track) {
          throw new Error("Webcam stream produced no video track");
        }
        videoTrack = track;
        liveStream = webcamStream;
      } else {
        throw new Error("No video source acquired");
      }

      const audioContext = new AudioContext();
      audioContextRef.current = audioContext;
      const audioSources = [screenStream, micStream].filter((stream): stream is MediaStream => stream !== null);
      const mergedAudioTrack = mergeAudioTracks(audioContext, audioSources);

      const combinedStream = new MediaStream([videoTrack, ...(mergedAudioTrack ? [mergedAudioTrack] : [])]);
      setPreviewStream(liveStream);

      const opfs = await startOpfsRecorder(`vrecnu-${Date.now()}.${selected.container}`);
      opfsRef.current = opfs;

      const recorder = new MediaRecorder(combinedStream, { mimeType: selected.mimeType });
      mediaRecorderRef.current = recorder;

      recorder.ondataavailable = (event: BlobEvent) => {
        if (event.data.size === 0) {
          return;
        }
        pendingWritesRef.current = pendingWritesRef.current.then(async () => {
          const buffer = await event.data.arrayBuffer();
          opfs.writeChunk(buffer);
        });
      };

      async function finalizeRecording(): Promise<void> {
        send({ type: "CHUNKS_FLUSHED" });
        await pendingWritesRef.current;

        const { fileName, bytesWritten } = await opfs.finalize();
        const durationMs = performance.now() - startTimeRef.current;
        setDiagnostics((prev) => ({ ...prev, opfsBytesWritten: bytesWritten, durationSec: durationMs / 1000 }));

        const root = await navigator.storage.getDirectory();
        const fileHandle = await root.getFileHandle(fileName);
        let fileBlob: Blob = await fileHandle.getFile();
        if (selected.container === "webm") {
          fileBlob = await patchWebmDuration(fileBlob, durationMs);
        }
        setResultBlob(fileBlob);

        if (pollRef.current !== null) {
          window.clearInterval(pollRef.current);
          pollRef.current = null;
        }
        opfs.terminate();
        setPreviewStream(null);
        stopAllTracks();
        send({ type: "FINALIZED" });
      }

      recorder.onstop = () => void finalizeRecording();

      startTimeRef.current = performance.now();
      recorder.start(RECORDER_TIMESLICE_MS);
      send({ type: "START" });

      pollRef.current = window.setInterval(() => {
        setDiagnostics((prev) => ({
          ...prev,
          opfsBytesWritten: opfs.getDiagnostics().bytesWritten,
          heapUsedBytes: readHeapUsedBytes(),
        }));
      }, DIAGNOSTICS_POLL_MS);
    },
    [send, stopAllTracks],
  );

  const stop = useCallback((): void => {
    send({ type: "STOP" });
    compositeRef.current?.stop();
    mediaRecorderRef.current?.stop();
  }, [send]);

  const reset = useCallback((): void => {
    stopAllTracks();
    compositeRef.current?.stop();
    compositeRef.current = null;
    opfsRef.current?.terminate();
    opfsRef.current = null;
    if (pollRef.current !== null) {
      window.clearInterval(pollRef.current);
      pollRef.current = null;
    }
    setPreviewStream(null);
    setResultBlob(null);
    setErrorMessage(null);
    setContainer(null);
    setDiagnostics(INITIAL_DIAGNOSTICS);
    send({ type: "RESET" });
  }, [send, stopAllTracks]);

  return { state, container, diagnostics, resultBlob, errorMessage, previewStream, start, stop, reset };
}
