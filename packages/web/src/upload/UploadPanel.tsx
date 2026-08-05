import { useState, type JSX } from "react";
import { Button } from "@vrecnu/ui";
import { GoogleDriveProvider, MockStorageProvider, type StorageProvider } from "@vrecnu/storage";
import { ConnectDriveButton, createGisTokenGetter, DRIVE_FILE_SCOPE } from "@vrecnu/publish";

export interface UploadPanelProps {
  file: Blob;
  fileName: string;
  mimeType: string;
}

type UploadStatus = "idle" | "uploading" | "done" | "error";

interface ProviderSetup {
  provider: StorageProvider;
  isDemo: boolean;
}

function buildProvider(): ProviderSetup {
  const clientId = import.meta.env.VITE_GOOGLE_CLIENT_ID;
  if (clientId) {
    return {
      provider: new GoogleDriveProvider({
        getAccessToken: createGisTokenGetter({ clientId, scope: DRIVE_FILE_SCOPE }),
      }),
      isDemo: false,
    };
  }
  return { provider: new MockStorageProvider(), isDemo: true };
}

/**
 * Wraps ConnectDriveButton with the resumable-upload -> share-link "magic moment" from
 * ARCHITECTURE.md §4. Falls back to MockStorageProvider (clearly labeled) when
 * VITE_GOOGLE_CLIENT_ID isn't set, so the full loop is testable before OAuth is registered.
 */
export function UploadPanel({ file, fileName, mimeType }: UploadPanelProps): JSX.Element {
  const [{ provider, isDemo }] = useState<ProviderSetup>(buildProvider);
  const [connected, setConnected] = useState(() => provider.isConnected());
  const [status, setStatus] = useState<UploadStatus>("idle");
  const [progress, setProgress] = useState(0);
  const [link, setLink] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleUpload = async (): Promise<void> => {
    setStatus("uploading");
    setProgress(0);
    setError(null);
    try {
      const { fileId } = await provider.uploadResumable(file, { name: fileName, mimeType }, (uploadProgress) => {
        setProgress(uploadProgress.totalBytes > 0 ? uploadProgress.bytesUploaded / uploadProgress.totalBytes : 0);
      });
      const shareLink = await provider.createShareLink(fileId);
      setLink(shareLink);
      setStatus("done");
      try {
        await navigator.clipboard.writeText(shareLink);
        setCopied(true);
      } catch {
        // Clipboard permission can be denied/unavailable — the manual Copy button below covers this.
      }
    } catch (err) {
      setError(err instanceof Error ? err.message : "Upload failed");
      setStatus("error");
    }
  };

  const handleCopy = async (): Promise<void> => {
    if (!link) {
      return;
    }
    try {
      await navigator.clipboard.writeText(link);
      setCopied(true);
    } catch {
      setError("Couldn't copy automatically — select the link text and copy it manually.");
    }
  };

  return (
    <div className="flex w-full max-w-xl flex-col gap-3 rounded-md border border-gray-200 p-4">
      {isDemo ? (
        <p className="text-sm text-amber-600">Demo mode — set VITE_GOOGLE_CLIENT_ID to upload to a real Google Drive.</p>
      ) : null}

      {!connected ? (
        <ConnectDriveButton provider={provider} onConnected={() => setConnected(true)} />
      ) : null}

      {connected && status === "idle" ? <Button onClick={() => void handleUpload()}>Upload to Google Drive</Button> : null}

      {status === "uploading" ? (
        <div className="flex flex-col gap-1">
          <div className="h-2 w-full overflow-hidden rounded bg-gray-100">
            <div className="h-full bg-blue-600 transition-all" style={{ width: `${Math.round(progress * 100)}%` }} />
          </div>
          <span className="text-sm text-gray-500">Uploading… {Math.round(progress * 100)}%</span>
        </div>
      ) : null}

      {status === "done" && link ? (
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-2">
            <input
              readOnly
              value={link}
              className="flex-1 rounded border border-gray-300 px-2 py-1 text-sm"
              onFocus={(event) => event.currentTarget.select()}
            />
            <Button size="sm" variant="outline" onClick={() => void handleCopy()}>
              {copied ? "Copied" : "Copy"}
            </Button>
          </div>
          <p className="text-sm text-green-600">Uploaded — link is shareable with anyone who has it.</p>
        </div>
      ) : null}

      {error ? <p className="text-sm text-red-600">{error}</p> : null}
    </div>
  );
}
