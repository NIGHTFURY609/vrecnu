import * as React from "react";
import { Button } from "@vrecnu/ui";
import { GoogleDriveProvider, type StorageProvider } from "@vrecnu/storage";
import { createGisTokenGetter, DRIVE_FILE_SCOPE } from "./gis-token-client";

export interface ConnectDriveButtonProps {
  /** Injected for tests / MockStorageProvider wiring; defaults to a real GoogleDriveProvider. */
  provider?: StorageProvider;
  onConnected?: (provider: StorageProvider) => void;
}

function buildDefaultProvider(): StorageProvider | null {
  const clientId = import.meta.env.VITE_GOOGLE_CLIENT_ID;
  if (!clientId) {
    return null;
  }
  return new GoogleDriveProvider({
    getAccessToken: createGisTokenGetter({ clientId, scope: DRIVE_FILE_SCOPE }),
  });
}

export function ConnectDriveButton({ provider, onConnected }: ConnectDriveButtonProps): React.JSX.Element {
  const [connecting, setConnecting] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const resolvedProvider = provider ?? buildDefaultProvider();

  if (!resolvedProvider) {
    return (
      <Button variant="outline" disabled title="Set VITE_GOOGLE_CLIENT_ID to enable Drive connect">
        Google Drive not configured
      </Button>
    );
  }

  const handleClick = async (): Promise<void> => {
    setConnecting(true);
    setError(null);
    try {
      await resolvedProvider.connect();
      onConnected?.(resolvedProvider);
    } catch (err) {
      setError(err instanceof Error ? err.message : "Failed to connect to Google Drive");
    } finally {
      setConnecting(false);
    }
  };

  return (
    <div className="flex flex-col gap-2">
      <Button onClick={() => void handleClick()} disabled={connecting}>
        {connecting ? "Connecting…" : "Connect Google Drive"}
      </Button>
      {error ? <p className="text-sm text-red-600">{error}</p> : null}
    </div>
  );
}
