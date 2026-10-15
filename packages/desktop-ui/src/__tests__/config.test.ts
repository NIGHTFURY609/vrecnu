import { describe, expect, it } from "vitest";
import { orderSources, reconcileConfig, sourceLabel, type Devices } from "../config";
import { DEFAULT_BUBBLE, type CaptureSource, type SessionConfig } from "../engine";

const monitor = (id: string, isPrimary: boolean): CaptureSource => ({
  id,
  title: `Display ${id}`,
  kind: "monitor",
  size: { width: 1920, height: 1080 },
  isPrimary,
});
const win = (id: string, title: string, width = 800): CaptureSource => ({
  id,
  title,
  kind: "window",
  size: { width, height: width === 0 ? 0 : 600 },
  isPrimary: false,
});

const devices: Devices = {
  sources: [win("window:2", "Zed"), monitor("monitor:2", false), win("window:1", "Browser"), monitor("monitor:1", true)],
  cameras: [{ id: "0", name: "USB Camera" }],
  audio: {
    inputs: [{ id: "default", name: "Default microphone", isDefault: true }],
    outputs: [{ id: "default", name: "System audio", isDefault: true }],
  },
};

const config: SessionConfig = {
  source: "window:1",
  camera: "0",
  microphone: "default",
  systemAudio: true,
  fps: 30,
  videoBitrate: 6_000_000,
  bubble: DEFAULT_BUBBLE,
};

describe("orderSources", () => {
  it("lists the primary screen first, then other screens, then windows by title", () => {
    expect(orderSources(devices.sources).map((s) => s.id)).toEqual(["monitor:1", "monitor:2", "window:1", "window:2"]);
  });
});

describe("sourceLabel", () => {
  it("shows a size when the engine knows one", () => {
    expect(sourceLabel(win("window:1", "Browser"))).toBe("Browser (800×600)");
    expect(sourceLabel(monitor("monitor:1", true))).toBe("Display monitor:1 - primary (1920×1080)");
  });

  it("omits the size of a minimized window rather than printing 0×0", () => {
    expect(sourceLabel(win("window:9", "Notes", 0))).toBe("Notes");
  });
});

describe("reconcileConfig", () => {
  it("keeps choices that still exist", () => {
    expect(reconcileConfig(config, devices)).toEqual(config);
  });

  it("falls back to the primary screen when the chosen window has closed", () => {
    expect(reconcileConfig({ ...config, source: "window:404" }, devices).source).toBe("monitor:1");
  });

  it("picks the primary screen when nothing was chosen yet", () => {
    expect(reconcileConfig({ ...config, source: null }, devices).source).toBe("monitor:1");
  });

  it("drops an unplugged camera, and the bubble with it", () => {
    const next = reconcileConfig({ ...config, camera: "7" }, devices);
    expect(next.camera).toBeNull();
    expect(next.bubble).toBeNull();
  });

  it("drops a microphone that is gone", () => {
    const next = reconcileConfig(config, { ...devices, audio: { ...devices.audio, inputs: [] } });
    expect(next.microphone).toBeNull();
  });

  it("turns system audio off when there is no output device to loop back", () => {
    const next = reconcileConfig(config, { ...devices, audio: { ...devices.audio, outputs: [] } });
    expect(next.systemAudio).toBe(false);
  });

  it("leaves frame rate and bitrate alone", () => {
    const next = reconcileConfig({ ...config, fps: 60, videoBitrate: 8_000_000 }, devices);
    expect([next.fps, next.videoBitrate]).toEqual([60, 8_000_000]);
  });
});
