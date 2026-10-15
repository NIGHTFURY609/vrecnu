// The only module that touches @tauri-apps/api.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppState, AudioDevices, CameraInfo, CaptureSource, EngineEvent } from "./engine";

/** Must match vrecnu-shell-tauri's events::ENGINE_EVENT. */
export const ENGINE_EVENT = "engine-event";

export const getSnapshot = () => invoke<AppState>("get_snapshot");
export const listSources = () => invoke<CaptureSource[]>("list_sources");
export const listCameras = () => invoke<CameraInfo[]>("list_cameras");
export const listAudioDevices = () => invoke<AudioDevices>("list_audio_devices");

export function onEngineEvent(handler: (event: EngineEvent) => void): Promise<UnlistenFn> {
  return listen<EngineEvent>(ENGINE_EVENT, (message) => handler(message.payload));
}
