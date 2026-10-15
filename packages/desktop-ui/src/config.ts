import type { AudioDevices, CameraInfo, CaptureSource, SessionConfig } from "./engine";

export interface Devices {
  sources: CaptureSource[];
  cameras: CameraInfo[];
  audio: AudioDevices;
}

/** The primary screen first, then other screens, then windows by title. */
export function orderSources(sources: CaptureSource[]): CaptureSource[] {
  const rank = (s: CaptureSource) => (s.kind === "monitor" ? (s.isPrimary ? 0 : 1) : 2);
  return [...sources].sort((a, b) => rank(a) - rank(b) || a.title.localeCompare(b.title));
}

/** A minimized window has no known size (the engine reports 0x0), so none is shown. */
export function sourceLabel(source: CaptureSource): string {
  const primary = source.isPrimary ? " - primary" : "";
  const size = source.size.width > 0 ? ` (${source.size.width}×${source.size.height})` : "";
  return `${source.title}${primary}${size}`;
}

/**
 * Drops choices this machine can no longer honour: a closed window, an unplugged camera, a
 * removed microphone. Kept, they fail at Record, or worse from the tray, with an error the user
 * cannot connect to a choice made earlier.
 */
export function reconcileConfig(config: SessionConfig, devices: Devices): SessionConfig {
  const fallback = orderSources(devices.sources)[0]?.id ?? null;
  const source = devices.sources.some((s) => s.id === config.source) ? config.source : fallback;
  const camera = devices.cameras.some((c) => c.id === config.camera) ? config.camera : null;
  const microphone = devices.audio.inputs.some((d) => d.id === config.microphone) ? config.microphone : null;
  return {
    ...config,
    source,
    camera,
    microphone,
    systemAudio: config.systemAudio && devices.audio.outputs.length > 0,
    bubble: camera === null ? null : config.bubble,
  };
}
