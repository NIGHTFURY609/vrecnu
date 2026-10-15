import { useEffect, useState } from "react";
import type { AudioDevices, CameraInfo, CaptureSource } from "./engine";
import { listAudioDevices, listCameras, listSources } from "./tauri";
import { useEngine } from "./useEngine";

interface Devices {
  sources: CaptureSource[];
  cameras: CameraInfo[];
  audio: AudioDevices;
}

type DevicesState = { status: "loading" } | { status: "ready"; devices: Devices } | { status: "error"; error: unknown };

/** Diagnostic screen for M6-S2: proves the IPC contract end to end. Recorder UI arrives in S3. */
export function App() {
  const store = useEngine();
  const [devices, setDevices] = useState<DevicesState>({ status: "loading" });

  useEffect(() => {
    let cancelled = false;
    Promise.all([listSources(), listCameras(), listAudioDevices()])
      .then(([sources, cameras, audio]) => {
        if (!cancelled) setDevices({ status: "ready", devices: { sources, cameras, audio } });
      })
      .catch((error: unknown) => {
        if (!cancelled) setDevices({ status: "error", error });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <main className="flex min-h-screen flex-col items-center justify-center gap-2 p-6 text-slate-100">
      <h1 className="text-xl font-semibold">vrecnu</h1>
      <section className="w-full max-w-xl">
        <h2 className="text-sm font-semibold text-slate-300">Engine state</h2>
        <pre className="overflow-auto rounded bg-slate-900 p-3 text-xs text-slate-300">
          {store.status === "ready" ? JSON.stringify(store.view.app, null, 2) : "connecting…"}
        </pre>
      </section>
      <section className="w-full max-w-xl">
        <h2 className="text-sm font-semibold text-slate-300">Devices</h2>
        <pre className="overflow-auto rounded bg-slate-900 p-3 text-xs text-slate-300">
          {devices.status === "loading" && "loading…"}
          {devices.status === "error" && JSON.stringify(devices.error, null, 2)}
          {devices.status === "ready" && deviceSummary(devices.devices)}
        </pre>
      </section>
    </main>
  );
}

/** Window titles are deliberately left out: they can carry private content. */
function deviceSummary({ sources, cameras, audio }: Devices): string {
  const lines = sources.map((s) => `${s.kind} ${s.id} ${s.size.width}x${s.size.height}`);
  return [
    `sources: ${sources.length}`,
    ...lines,
    `cameras: ${cameras.length}`,
    `microphones: ${audio.inputs.length}`,
    `outputs: ${audio.outputs.length}`,
  ].join("\n");
}
