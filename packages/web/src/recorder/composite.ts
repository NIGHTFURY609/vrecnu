/**
 * Canvas compositor proving ADR-003 §3.4: the draw loop is clocked off the screen video's
 * real frames via requestVideoFrameCallback, never rAF/setInterval. canvas.captureStream(0)
 * puts the output track in manual mode — captureTrack.requestFrame() is called exactly once
 * per composited frame, so the output stream is locked to the source, not to display refresh.
 */

const WEBCAM_BUBBLE_WIDTH_RATIO = 0.22;
const WEBCAM_BUBBLE_MARGIN_PX = 16;
const DEFAULT_WEBCAM_ASPECT = 0.75; // 4:3 fallback until the webcam video reports real dimensions

export interface CompositeDiagnostics {
  compositedFrames: number;
  droppedVideoFrames: number;
  lastFrameAt: number;
}

export interface StartCompositeOptions {
  screenVideo: HTMLVideoElement;
  webcamVideo: HTMLVideoElement | null;
  width: number;
  height: number;
  onFrame?: (diagnostics: CompositeDiagnostics) => void;
}

export interface CompositeHandle {
  readonly canvas: HTMLCanvasElement;
  readonly stream: MediaStream;
  stop: () => void;
  getDiagnostics: () => CompositeDiagnostics;
}

export function startComposite(options: StartCompositeOptions): CompositeHandle {
  const { screenVideo, webcamVideo, width, height, onFrame } = options;

  const canvas = document.createElement("canvas");
  canvas.width = width;
  canvas.height = height;
  const context = canvas.getContext("2d");
  if (!context) {
    throw new Error("Failed to acquire 2D canvas context for compositing");
  }

  const stream = canvas.captureStream(0);
  const captureTrack = stream.getVideoTracks()[0] as CanvasCaptureMediaStreamTrack | undefined;
  if (!captureTrack) {
    throw new Error("captureStream(0) produced no video track");
  }

  const diagnostics: CompositeDiagnostics = {
    compositedFrames: 0,
    droppedVideoFrames: 0,
    lastFrameAt: 0,
  };

  let stopped = false;
  let rvfcHandle: number | null = null;

  function drawFrame(now: DOMHighResTimeStamp): void {
    if (stopped) {
      return;
    }

    // Non-null: both were validated once, above, before the first drawFrame was ever scheduled.
    context!.drawImage(screenVideo, 0, 0, width, height);

    if (webcamVideo && webcamVideo.readyState >= HTMLMediaElement.HAVE_CURRENT_DATA) {
      const bubbleWidth = width * WEBCAM_BUBBLE_WIDTH_RATIO;
      const aspect = webcamVideo.videoWidth > 0 ? webcamVideo.videoHeight / webcamVideo.videoWidth : DEFAULT_WEBCAM_ASPECT;
      const bubbleHeight = bubbleWidth * aspect;
      const x = width - bubbleWidth - WEBCAM_BUBBLE_MARGIN_PX;
      const y = height - bubbleHeight - WEBCAM_BUBBLE_MARGIN_PX;
      context!.drawImage(webcamVideo, x, y, bubbleWidth, bubbleHeight);
    }

    captureTrack!.requestFrame();
    diagnostics.compositedFrames += 1;
    diagnostics.droppedVideoFrames = screenVideo.getVideoPlaybackQuality?.().droppedVideoFrames ?? 0;
    diagnostics.lastFrameAt = now;
    onFrame?.({ ...diagnostics });

    rvfcHandle = screenVideo.requestVideoFrameCallback(drawFrame);
  }

  rvfcHandle = screenVideo.requestVideoFrameCallback(drawFrame);

  return {
    canvas,
    stream,
    stop: () => {
      stopped = true;
      if (rvfcHandle !== null) {
        screenVideo.cancelVideoFrameCallback(rvfcHandle);
      }
    },
    getDiagnostics: () => ({ ...diagnostics }),
  };
}
