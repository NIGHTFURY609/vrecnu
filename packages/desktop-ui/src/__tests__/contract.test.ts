import { describe, expect, it } from "vitest";
import fixture from "../__fixtures__/engine.json";
import {
  DEFAULT_BUBBLE,
  type AppError,
  type AppState,
  type AudioDevice,
  type AudioDevices,
  type BubbleSpec,
  type CameraInfo,
  type CaptureSource,
  type CaptureStats,
  type EngineEvent,
  type EngineEventType,
  type Recording,
  type SessionConfig,
} from "../engine";
import { initialStore, reduce } from "../state";

const data = fixture as unknown as {
  events: EngineEvent[];
  appState: AppState;
  sources: CaptureSource[];
  cameras: CameraInfo[];
  audioDevices: AudioDevices;
  sessionConfig: SessionConfig;
};

const EVENT_KEYS = {
  stateChanged: ["type", "from", "to"],
  elapsed: ["type", "ms"],
  audioLevels: ["type", "micDb", "systemDb"],
  stats: ["type", "framesIn", "framesWritten", "framesDropped", "framesDuplicated", "encoderFps"],
  warning: ["type", "code", "message"],
  renderProgress: ["type", "id", "fraction"],
  renderDone: ["type", "id", "recording"],
  publishOpened: ["type", "url", "port"],
  publishProgress: ["type", "bytesSent", "total"],
  publishDone: ["type", "webViewLink", "fileId"],
  publishFailed: ["type", "error"],
  failed: ["type", "error"],
} as const satisfies { [K in EngineEventType]: readonly (keyof Extract<EngineEvent, { type: K }>)[] };

const APP_STATE_KEYS = [
  "session",
  "sessionId",
  "elapsedMs",
  "stats",
  "recording",
  "error",
  "publishing",
] as const satisfies readonly (keyof AppState)[];
const APP_ERROR_KEYS = [
  "code",
  "message",
  "hint",
  "detail",
  "recoverable",
  "sessionLost",
] as const satisfies readonly (keyof AppError)[];
const RECORDING_KEYS = [
  "session",
  "path",
  "durationMs",
  "sizeBytes",
  "mimeType",
] as const satisfies readonly (keyof Recording)[];
const STATS_KEYS = [
  "framesIn",
  "framesWritten",
  "framesDropped",
  "framesDuplicated",
  "encoderFps",
] as const satisfies readonly (keyof CaptureStats)[];
const SOURCE_KEYS = [
  "id",
  "title",
  "kind",
  "size",
  "isPrimary",
] as const satisfies readonly (keyof CaptureSource)[];
const CAMERA_KEYS = ["id", "name"] as const satisfies readonly (keyof CameraInfo)[];
const AUDIO_DEVICE_KEYS = ["id", "name", "isDefault"] as const satisfies readonly (keyof AudioDevice)[];
const SESSION_CONFIG_KEYS = [
  "source",
  "camera",
  "microphone",
  "systemAudio",
  "fps",
  "videoBitrate",
  "bubble",
] as const satisfies readonly (keyof SessionConfig)[];
const BUBBLE_KEYS = ["diameter", "position", "clickThrough"] as const satisfies readonly (keyof BubbleSpec)[];

type MissingConfig = Exclude<keyof SessionConfig, (typeof SESSION_CONFIG_KEYS)[number]>;
const noConfigFieldUnlisted: [MissingConfig] extends [never] ? true : false = true;

// Compile-time completeness: a field added to the mirror but not listed above fails `pnpm typecheck`.
type MissingEvent = {
  [K in EngineEventType]: Exclude<keyof Extract<EngineEvent, { type: K }>, (typeof EVENT_KEYS)[K][number]>;
}[EngineEventType];
const noEventFieldUnlisted: [MissingEvent] extends [never] ? true : false = true;

type MissingApp = Exclude<keyof AppState, (typeof APP_STATE_KEYS)[number]>;
const noAppFieldUnlisted: [MissingApp] extends [never] ? true : false = true;

const sorted = (xs: readonly string[]) => [...xs].sort();

describe("engine.json contract", () => {
  it("lists every mirror field (compile-time)", () => {
    expect(noEventFieldUnlisted).toBe(true);
    expect(noAppFieldUnlisted).toBe(true);
  });

  it("covers every event type exactly once", () => {
    expect(sorted(data.events.map((e) => e.type))).toEqual(sorted(Object.keys(EVENT_KEYS)));
  });

  it.each(data.events.map((e) => [e.type, e] as const))("event %s has exactly the mirrored keys", (type, event) => {
    expect(sorted(Object.keys(event))).toEqual(sorted(EVENT_KEYS[type]));
  });

  it("matches the object shapes", () => {
    const failed = data.events.find((e) => e.type === "failed");
    if (failed?.type !== "failed") throw new Error("fixture lacks a failed event");
    expect(sorted(Object.keys(data.appState))).toEqual(sorted(APP_STATE_KEYS));
    expect(sorted(Object.keys(data.appState.stats))).toEqual(sorted(STATS_KEYS));
    expect(data.appState.recording).not.toBeNull();
    expect(sorted(Object.keys(data.appState.recording ?? {}))).toEqual(sorted(RECORDING_KEYS));
    expect(sorted(Object.keys(failed.error))).toEqual(sorted(APP_ERROR_KEYS));
    expect(sorted(Object.keys(data.sources[0] ?? {}))).toEqual(sorted(SOURCE_KEYS));
    expect(sorted(Object.keys(data.cameras[0] ?? {}))).toEqual(sorted(CAMERA_KEYS));
    expect(sorted(Object.keys(data.audioDevices.inputs[0] ?? {}))).toEqual(sorted(AUDIO_DEVICE_KEYS));
  });

  it("matches the session config the UI sends back through configure", () => {
    expect(noConfigFieldUnlisted).toBe(true);
    expect(sorted(Object.keys(data.sessionConfig))).toEqual(sorted(SESSION_CONFIG_KEYS));
    expect(sorted(Object.keys(data.sessionConfig.bubble ?? {}))).toEqual(sorted(BUBBLE_KEYS));
    // A Rust tuple: an array on the wire, not {x, y}.
    expect(data.sessionConfig.bubble?.position).toHaveLength(2);
  });

  it("DEFAULT_BUBBLE mirrors BubbleSpec::default()", () => {
    expect(DEFAULT_BUBBLE).toEqual(data.sessionConfig.bubble);
  });

  describe("reducer applies each fixture event on top of the snapshot", () => {
    const seeded = reduce(initialStore, { kind: "snapshot", state: data.appState });
    const applyOne = (type: EngineEventType) => {
      const event = data.events.find((e) => e.type === type);
      if (!event) throw new Error(`fixture lacks ${type}`);
      const next = reduce(seeded, { kind: "event", event });
      if (next.status !== "ready") throw new Error("expected ready");
      return next.view;
    };

    it("stateChanged", () => expect(applyOne("stateChanged").app.session).toBe("recording"));
    it("elapsed", () => expect(applyOne("elapsed").app.elapsedMs).toBe(1500));
    it("audioLevels", () => expect(applyOne("audioLevels").levels).toEqual({ micDb: -12.5, systemDb: -30.25 }));
    it("stats", () => {
      const v = applyOne("stats");
      expect(v.app.stats.framesDuplicated).toBe(12);
      expect(v.app.stats.encoderFps).toBe(29.5);
    });
    it("warning", () => expect(applyOne("warning").warnings[0]?.code).toBe("CAPTURE_BACKPRESSURE"));
    it("renderProgress", () => expect(applyOne("renderProgress").render).toEqual({ id: 7, fraction: 0.25 }));
    it("renderDone", () => expect(applyOne("renderDone").app.recording?.mimeType).toBe("video/mp4"));
    it("publishOpened", () => expect(applyOne("publishOpened").app.publishing).toBe(true));
    it("publishProgress", () =>
      expect(applyOne("publishProgress").publish).toEqual({ phase: "uploading", bytesSent: 1024, total: 4096 }));
    it("publishDone", () => {
      const p = applyOne("publishDone").publish;
      expect(p.phase === "done" && p.webViewLink).toBe("https://drive.google.com/file/d/FILE_ID/view");
    });
    it("publishFailed", () => {
      const p = applyOne("publishFailed").publish;
      expect(p.phase).toBe("failed");
      expect(p.phase === "failed" && p.error.sessionLost).toBe(false);
    });
    it("failed", () => expect(applyOne("failed").app.error?.code).toBe("PUBLISH_PORTS_BUSY"));
  });
});
