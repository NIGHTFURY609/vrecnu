import { useEffect, useState } from "react";
import { Button } from "@vrecnu/ui";
import { orderSources, reconcileConfig, sourceLabel, type Devices } from "../config";
import { DEFAULT_BUBBLE, type AppError, type SessionConfig } from "../engine";
import { toAppError } from "../errors";
import { configure, getConfig, listAudioDevices, listCameras, listSources, startRecording } from "../tauri";
import { ErrorNote } from "./ErrorNote";
import { DANGER, FIELD } from "./styles";

type Setup =
  | { status: "loading" }
  | { status: "ready"; devices: Devices; config: SessionConfig }
  | { status: "error"; error: AppError };

export function Recorder() {
  const [setup, setSetup] = useState<Setup>({ status: "loading" });
  const [starting, setStarting] = useState(false);
  const [error, setError] = useState<AppError | null>(null);

  useEffect(() => {
    let cancelled = false;
    Promise.all([listSources(), listCameras(), listAudioDevices(), getConfig()])
      .then(async ([sources, cameras, audio, stored]) => {
        const devices = { sources, cameras, audio };
        const config = reconcileConfig(stored, devices);
        // Stored straight back, so the tray's "Start recording" cannot use a stale choice either.
        await configure(config);
        if (!cancelled) setSetup({ status: "ready", devices, config });
      })
      .catch((err: unknown) => {
        if (!cancelled) setSetup({ status: "error", error: toAppError(err) });
      });
    return () => {
      cancelled = true;
    };
  }, []);

  if (setup.status === "loading") {
    return <p className="text-sm text-slate-400">Looking for screens, cameras and microphones…</p>;
  }
  if (setup.status === "error") return <ErrorNote error={setup.error} />;

  const { devices, config } = setup;
  const sources = orderSources(devices.sources);
  const monitors = sources.filter((s) => s.kind === "monitor");
  const windows = sources.filter((s) => s.kind === "window");
  const mic = devices.audio.inputs[0] ?? null;
  const hasOutput = devices.audio.outputs.length > 0;

  const change = (patch: Partial<SessionConfig>) => {
    const next = { ...config, ...patch };
    setSetup({ status: "ready", devices, config: next });
    // Stored on every change, not only at Record, so the tray records what this window shows.
    configure(next).catch((err: unknown) => setError(toAppError(err)));
  };

  const record = () => {
    setStarting(true);
    setError(null);
    // On success the shell destroys this window as the session starts (spec D3).
    configure(config)
      .then(() => startRecording())
      .catch((err: unknown) => {
        setError(toAppError(err));
        setStarting(false);
      });
  };

  return (
    <form
      className="flex flex-col gap-5"
      onSubmit={(event) => {
        event.preventDefault();
        record();
      }}
    >
      <label className="flex flex-col gap-1 text-sm">
        <span className="text-slate-300">Record</span>
        <select
          className={FIELD}
          value={config.source ?? ""}
          disabled={sources.length === 0}
          onChange={(event) => change({ source: event.target.value })}
        >
          {sources.length === 0 && <option value="">No screens found</option>}
          <optgroup label="Screens">
            {monitors.map((s) => (
              <option key={s.id} value={s.id}>
                {sourceLabel(s)}
              </option>
            ))}
          </optgroup>
          {windows.length > 0 && (
            <optgroup label="Windows">
              {windows.map((s) => (
                <option key={s.id} value={s.id}>
                  {sourceLabel(s)}
                </option>
              ))}
            </optgroup>
          )}
        </select>
      </label>

      <fieldset className="flex flex-col gap-2 text-sm">
        <legend className="mb-1 text-slate-300">Sound</legend>
        <Toggle
          label={mic ? "Microphone" : "Microphone (none found)"}
          checked={config.microphone !== null}
          disabled={mic === null}
          onChange={(on) => change({ microphone: on && mic ? mic.id : null })}
        />
        <Toggle
          label={hasOutput ? "System audio" : "System audio (no output device)"}
          checked={config.systemAudio}
          disabled={!hasOutput}
          onChange={(on) => change({ systemAudio: on })}
        />
      </fieldset>

      <fieldset className="flex flex-col gap-2 text-sm">
        <legend className="mb-1 text-slate-300">Camera</legend>
        <select
          className={FIELD}
          value={config.camera ?? ""}
          onChange={(event) => {
            const camera = event.target.value === "" ? null : event.target.value;
            change({ camera, bubble: camera === null ? null : config.bubble });
          }}
        >
          <option value="">No camera</option>
          {devices.cameras.map((c) => (
            <option key={c.id} value={c.id}>
              {c.name}
            </option>
          ))}
        </select>
        <Toggle
          label="Show the camera bubble on screen"
          checked={config.bubble !== null}
          disabled={config.camera === null}
          onChange={(on) => change({ bubble: on ? DEFAULT_BUBBLE : null })}
        />
      </fieldset>

      {error && <ErrorNote error={error} />}

      <Button
        type="submit"
        variant="destructive"
        size="lg"
        className={DANGER}
        disabled={starting || config.source === null}
      >
        {starting ? "Starting…" : "Record"}
      </Button>
    </form>
  );
}

function Toggle({
  label,
  checked,
  disabled,
  onChange,
}: {
  label: string;
  checked: boolean;
  disabled: boolean;
  onChange: (on: boolean) => void;
}) {
  return (
    <label className={`flex items-center gap-2 ${disabled ? "text-slate-500" : "text-slate-100"}`}>
      <input
        type="checkbox"
        className="size-4 accent-red-600"
        checked={checked}
        disabled={disabled}
        onChange={(event) => onChange(event.target.checked)}
      />
      {label}
    </label>
  );
}
