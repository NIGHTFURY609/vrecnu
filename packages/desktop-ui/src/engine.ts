// TS mirror of vrecnu-shell-api / vrecnu-types serde shapes. Rust writes
// __fixtures__/engine.json; __tests__/contract.test.ts fails if this file disagrees with it.

export type SessionState = "idle" | "ready" | "recording" | "finalizing" | "stopped" | "failed";

export type ErrorCode =
  | "CAPTURE_UNAVAILABLE" | "CAPTURE_PERMISSION" | "CAPTURE_SOURCE_GONE" | "CAPTURE_BACKPRESSURE"
  | "AUDIO_NO_LOOPBACK" | "AUDIO_DEVICE_LOST" | "CAMERA_BUSY" | "CAMERA_NONE" | "CAMERA_PERMISSION"
  | "ENCODE_MISSING_BINARY" | "ENCODE_FAILED" | "ENCODE_DIED" | "PUBLISH_PORTS_BUSY"
  | "PUBLISH_ASSETS_MISSING" | "PUBLISH_TIMEOUT" | "PUBLISH_PAGE_ERROR" | "DISK_FULL"
  | "ILLEGAL_STATE" | "INTERNAL";

export interface AppError {
  code: ErrorCode;
  message: string;
  hint: string | null;
  detail: string | null;
  recoverable: boolean;
  sessionLost: boolean;
}

export interface CaptureStats {
  framesIn: number;
  framesWritten: number;
  framesDropped: number;
  framesDuplicated: number;
  encoderFps: number;
}

export interface Recording {
  session: number;
  path: string;
  durationMs: number;
  sizeBytes: number;
  mimeType: string;
}

export interface AppState {
  session: SessionState;
  sessionId: number | null;
  elapsedMs: number;
  stats: CaptureStats;
  recording: Recording | null;
  error: AppError | null;
  publishing: boolean;
}

export interface CaptureSource {
  id: string;
  title: string;
  kind: "monitor" | "window";
  size: { width: number; height: number };
  isPrimary: boolean;
}

export interface CameraInfo {
  id: string;
  name: string;
}

export interface AudioDevice {
  id: string;
  name: string;
  isDefault: boolean;
}

export interface AudioDevices {
  inputs: AudioDevice[];
  outputs: AudioDevice[];
}

export interface BubbleSpec {
  diameter: number;
  /** A Rust `(i32, i32)`: an array on the wire. */
  position: [number, number];
  clickThrough: boolean;
}

export interface SessionConfig {
  source: string | null;
  camera: string | null;
  microphone: string | null;
  systemAudio: boolean;
  fps: number;
  videoBitrate: number;
  bubble: BubbleSpec | null;
}

/** BubbleSpec::default() in vrecnu-types; contract.test.ts pins the two together. */
export const DEFAULT_BUBBLE: BubbleSpec = { diameter: 240, position: [48, 48], clickThrough: true };

export type EngineEvent =
  | { type: "stateChanged"; from: SessionState; to: SessionState }
  | { type: "elapsed"; ms: number }
  | { type: "audioLevels"; micDb: number; systemDb: number }
  | ({ type: "stats" } & CaptureStats)
  | { type: "warning"; code: ErrorCode; message: string }
  | { type: "renderProgress"; id: number; fraction: number }
  | { type: "renderDone"; id: number; recording: Recording }
  | { type: "publishOpened"; url: string; port: number }
  | { type: "publishProgress"; bytesSent: number; total: number }
  | { type: "publishDone"; webViewLink: string; fileId: string }
  | { type: "publishFailed"; error: AppError }
  | { type: "failed"; error: AppError };

export type EngineEventType = EngineEvent["type"];
