// The only module that touches @tauri-apps/api.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type { AppState, AudioDevices, CameraInfo, CaptureSource, EngineEvent, SessionConfig } from "./engine";

/** Must match vrecnu-shell-tauri's events::ENGINE_EVENT. */
export const ENGINE_EVENT = "engine-event";

export const getSnapshot = () => invoke<AppState>("get_snapshot");
export const listSources = () => invoke<CaptureSource[]>("list_sources");
export const listCameras = () => invoke<CameraInfo[]>("list_cameras");
export const listAudioDevices = () => invoke<AudioDevices>("list_audio_devices");

export const getConfig = () => invoke<SessionConfig>("get_config");
export const configure = (config: SessionConfig) => invoke<void>("configure", { config });
/** Resolves with the session id. The window closes as the session starts (spec D3). */
export const startRecording = () => invoke<number>("start_recording");
export const stopRecording = () => invoke<void>("stop_recording");
export const resetSession = () => invoke<void>("reset");
export const revealRecording = () => invoke<void>("reveal_recording");

export function onEngineEvent(handler: (event: EngineEvent) => void): Promise<UnlistenFn> {
  return listen<EngineEvent>(ENGINE_EVENT, (message) => handler(message.payload));
}
