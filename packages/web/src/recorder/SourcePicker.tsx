import { useEffect, useState, type JSX } from "react";
import { Button } from "@vrecnu/ui";

export interface RecordingSourceConfig {
  screen: boolean;
  webcam: boolean;
  mic: boolean;
  cameraDeviceId?: string | undefined;
  micDeviceId?: string | undefined;
}

export interface SourcePickerProps {
  value: RecordingSourceConfig;
  onChange: (config: RecordingSourceConfig) => void;
  disabled?: boolean;
}

interface DeviceOption {
  deviceId: string;
  label: string;
}

/** Grants a throwaway permission prompt so enumerateDevices() returns real labels, then releases it immediately. */
async function primeDeviceLabels(): Promise<void> {
  const stream = await navigator.mediaDevices.getUserMedia({ video: true, audio: true });
  stream.getTracks().forEach((track) => track.stop());
}

export function SourcePicker({ value, onChange, disabled }: SourcePickerProps): JSX.Element {
  const [cameras, setCameras] = useState<DeviceOption[]>([]);
  const [mics, setMics] = useState<DeviceOption[]>([]);
  const [labelsGranted, setLabelsGranted] = useState(false);
  const [priming, setPriming] = useState(false);

  const refreshDevices = async (): Promise<void> => {
    const devices = await navigator.mediaDevices.enumerateDevices();
    const cameraOptions = devices
      .filter((device) => device.kind === "videoinput")
      .map((device) => ({ deviceId: device.deviceId, label: device.label }));
    const micOptions = devices
      .filter((device) => device.kind === "audioinput")
      .map((device) => ({ deviceId: device.deviceId, label: device.label }));
    setCameras(cameraOptions);
    setMics(micOptions);
    setLabelsGranted(cameraOptions.some((device) => device.label !== "") || micOptions.some((device) => device.label !== ""));
  };

  useEffect(() => {
    // One-time mount fetch with no deps to race — react-hooks/set-state-in-effect's
    // interprocedural check flags any effect that transitively calls a setState setter,
    // including this safe async-fetch-then-setState idiom.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void refreshDevices();
  }, []);

  const handleGrantAccess = async (): Promise<void> => {
    setPriming(true);
    try {
      await primeDeviceLabels();
      await refreshDevices();
    } catch {
      // Best-effort — if the user denies, the Webcam/Mic toggles will fail loudly at record time.
    } finally {
      setPriming(false);
    }
  };

  const update = (patch: Partial<RecordingSourceConfig>): void => {
    onChange({ ...value, ...patch });
  };

  const noVideoSourceSelected = !value.screen && !value.webcam;

  return (
    <div className="flex flex-col gap-4 rounded-md border border-gray-200 p-4">
      <div className="flex flex-col gap-2">
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={value.screen}
            disabled={disabled}
            onChange={(event) => update({ screen: event.target.checked })}
          />
          Screen
        </label>

        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={value.webcam}
            disabled={disabled}
            onChange={(event) => update({ webcam: event.target.checked })}
          />
          Webcam
        </label>
        {value.webcam ? (
          <select
            className="ml-6 rounded border border-gray-300 px-2 py-1 text-sm"
            disabled={disabled}
            value={value.cameraDeviceId ?? ""}
            onChange={(event) => update({ cameraDeviceId: event.target.value || undefined })}
          >
            <option value="">Default camera</option>
            {cameras.map((camera) => (
              <option key={camera.deviceId} value={camera.deviceId}>
                {camera.label || camera.deviceId}
              </option>
            ))}
          </select>
        ) : null}

        <label className="flex items-center gap-2 text-sm">
          <input type="checkbox" checked={value.mic} disabled={disabled} onChange={(event) => update({ mic: event.target.checked })} />
          Microphone
        </label>
        {value.mic ? (
          <select
            className="ml-6 rounded border border-gray-300 px-2 py-1 text-sm"
            disabled={disabled}
            value={value.micDeviceId ?? ""}
            onChange={(event) => update({ micDeviceId: event.target.value || undefined })}
          >
            <option value="">Default microphone</option>
            {mics.map((mic) => (
              <option key={mic.deviceId} value={mic.deviceId}>
                {mic.label || mic.deviceId}
              </option>
            ))}
          </select>
        ) : null}
      </div>

      {!labelsGranted && (
        <Button size="sm" variant="outline" disabled={Boolean(disabled) || priming} onClick={() => void handleGrantAccess()}>
          {priming ? "Requesting…" : "Show device names"}
        </Button>
      )}

      {noVideoSourceSelected ? <p className="text-sm text-red-600">Select Screen and/or Webcam to record.</p> : null}
    </div>
  );
}
