import { useCallback, useRef, useState, type JSX } from "react";
import { Button } from "@vrecnu/ui";
import {
  blobByteSource,
  GoogleDriveProvider,
  MockStorageProvider,
  UploadError,
  type ByteSource,
  type ResumableSession,
  type StorageProvider,
} from "@vrecnu/storage";
import { ConnectDriveButton } from "./connect-drive-button";
import { createGisTokenGetter, DRIVE_FILE_SCOPE } from "./gis-token-client";
import { resolveCredentials } from "./credentials";

export interface UploadPanelProps {
  /** A Blob (PWA/OPFS) or a ByteSource (desktop loopback). Neither is read eagerly. */
  source: Blob | ByteSource;
  fileName: string;
  mimeType: string;
  /**
   * Set when the recording's container has no valid duration. AGENTS.md §10 forbids
   * uploading one silently — it produces a file Drive's preview player cannot seek.
   */
  durationWarning?: string | null;
  /** Called with the share link once the upload completes. */
  onComplete?: (result: { fileId: string; webViewLink: string }) => void;
  onFailed?: (message: string) => void;
  onProgress?: (fraction: number) => void;
}

type UploadStatus = "idle" | "uploading" | "done" | "error";

interface ProviderSetup {
  provider: StorageProvider;
  isDemo: boolean;
}

/** Session URIs outlive the page, so a reload can resume rather than restart. */
const SESSION_STORAGE_KEY = "vrecnu.upload.session";
/** Drive expires resumable sessions after about a week. */
const SESSION_MAX_AGE_MS = 6 * 24 * 60 * 60 * 1000;

function loadPersistedSession(fileName: string, fileSize: number): ResumableSession | null {
  try {
    const raw = window.localStorage.getItem(SESSION_STORAGE_KEY);
    if (!raw) {
      return null;
    }
    const session = JSON.parse(raw) as ResumableSession;
    // Only resume a session that belongs to *this* file; a stale one would corrupt it.
    if (session.fileName !== fileName || session.fileSize !== fileSize) {
      return null;
    }
    if (Date.now() - session.createdAt > SESSION_MAX_AGE_MS) {
      return null;
    }
    return session;
  } catch {
    return null;
  }
}

function persistSession(session: ResumableSession): void {
  try {
    window.localStorage.setItem(SESSION_STORAGE_KEY, JSON.stringify(session));
  } catch {
    // Private mode or a full quota. Resume-after-reload is a bonus, not a requirement.
  }
}

function forgetSession(): void {
  try {
    window.localStorage.removeItem(SESSION_STORAGE_KEY);
  } catch {
    // Nothing actionable.
  }
}

function buildProvider(buildTimeClientId?: string): ProviderSetup {
  const { clientId } = resolveCredentials(buildTimeClientId);
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
 * The record -> upload -> link "magic moment" from ARCHITECTURE.md §4.
 *
 * This lives in `packages/publish` rather than in the web app because ADR-002 makes it the
 * *one* shared artifact between Phase 1 and Phase 2: the PWA renders it directly, and the
 * desktop app serves the same built page over its loopback origin. There is exactly one
 * OAuth + resumable-upload implementation, written and tested once.
 */
export function UploadPanel({
  source,
  fileName,
  mimeType,
  durationWarning,
  onComplete,
  onFailed,
  onProgress,
}: UploadPanelProps): JSX.Element {
  const [{ provider, isDemo }] = useState<ProviderSetup>(() =>
    buildProvider(import.meta.env.VITE_GOOGLE_CLIENT_ID),
  );
  const [connected, setConnected] = useState(() => provider.isConnected());
  const [status, setStatus] = useState<UploadStatus>("idle");
  const [progress, setProgress] = useState(0);
  const [link, setLink] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [durationAcknowledged, setDurationAcknowledged] = useState(false);
  const abortRef = useRef<AbortController | null>(null);

  const size = source instanceof Blob ? source.size : source.size;
  const blockedByDuration = Boolean(durationWarning) && !durationAcknowledged;

  const runUpload = useCallback(
    async (resumeUri?: string): Promise<void> => {
      const controller = new AbortController();
      abortRef.current = controller;
      setStatus("uploading");
      setError(null);

      try {
        const { fileId } = await provider.uploadResumable(
          source instanceof Blob ? blobByteSource(source) : source,
          { name: fileName, mimeType },
          {
            signal: controller.signal,
            ...(resumeUri ? { resumeSessionUri: resumeUri } : {}),
            onSessionStart: persistSession,
            onSessionEnd: forgetSession,
            onProgress: ({ bytesUploaded, totalBytes }) => {
              const fraction = totalBytes > 0 ? bytesUploaded / totalBytes : 0;
              setProgress(fraction);
              onProgress?.(fraction);
            },
          },
        );
        const webViewLink = await provider.createShareLink(fileId);
        setLink(webViewLink);
        setStatus("done");
        onComplete?.({ fileId, webViewLink });
        try {
          await navigator.clipboard.writeText(webViewLink);
          setCopied(true);
        } catch {
          // Clipboard permission can be denied/unavailable — the manual Copy button covers it.
        }
      } catch (err) {
        if (err instanceof UploadError && err.sessionExpired) {
          forgetSession();
        }
        const message = err instanceof Error ? err.message : "Upload failed";
        setError(message);
        setStatus("error");
        onFailed?.(message);
      } finally {
        abortRef.current = null;
      }
    },
    [source, fileName, mimeType, provider, onComplete, onFailed, onProgress],
  );

  const startUpload = (): void => {
    setProgress(0);
    void runUpload(loadPersistedSession(fileName, size)?.uri);
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

  const percent = Math.round(progress * 100);

  return (
    <div className="flex w-full max-w-xl flex-col gap-3 rounded-md border border-gray-200 p-4">
      {isDemo ? (
        <p className="text-sm text-amber-700" role="status">
          <strong>Demo mode.</strong> Nothing is really uploaded and the link below will not work. Connect a
          Google client ID to upload to a real Google Drive.
        </p>
      ) : null}

      {durationWarning ? (
        <div className="rounded border border-amber-300 bg-amber-50 p-2 text-sm text-amber-900">
          <p>{durationWarning}</p>
          {!durationAcknowledged ? (
            <Button size="sm" variant="outline" className="mt-2" onClick={() => setDurationAcknowledged(true)}>
              Upload anyway
            </Button>
          ) : null}
        </div>
      ) : null}

      {!connected ? <ConnectDriveButton provider={provider} onConnected={() => setConnected(true)} /> : null}

      {connected && status === "idle" ? (
        <Button onClick={startUpload} disabled={blockedByDuration}>
          Upload to Google Drive
        </Button>
      ) : null}

      {status === "uploading" ? (
        <div className="flex flex-col gap-1">
          <div
            className="h-2 w-full overflow-hidden rounded bg-gray-100"
            role="progressbar"
            aria-valuenow={percent}
            aria-valuemin={0}
            aria-valuemax={100}
            aria-label="Upload progress"
          >
            <div className="h-full bg-blue-600 transition-all" style={{ width: `${percent}%` }} />
          </div>
          <div className="flex items-center gap-2">
            <span className="text-sm text-gray-500" role="status">
              Uploading… {percent}%
            </span>
            <Button size="sm" variant="outline" onClick={() => abortRef.current?.abort()}>
              Cancel
            </Button>
          </div>
        </div>
      ) : null}

      {status === "done" && link ? (
        <div className="flex flex-col gap-2">
          <div className="flex items-center gap-2">
            <label className="sr-only" htmlFor="vrecnu-share-link">
              Shareable link
            </label>
            <input
              id="vrecnu-share-link"
              readOnly
              value={link}
              className="flex-1 rounded border border-gray-300 px-2 py-1 text-sm"
              onFocus={(event) => event.currentTarget.select()}
            />
            <Button size="sm" variant="outline" onClick={() => void handleCopy()}>
              {copied ? "Copied" : "Copy"}
            </Button>
          </div>
          <p className="text-sm text-green-600" role="status">
            {isDemo ? "Demo mode — this link is not real." : "Uploaded — anyone with the link can view it."}
          </p>
        </div>
      ) : null}

      {status === "error" ? (
        <div className="flex items-center gap-2">
          {/* The panel used to have no way back from an error: `status` never returned to
              idle, so the Upload button never reappeared and the only escape discarded the
              recording. Retry resumes from the server's offset where a session survives. */}
          <Button size="sm" onClick={startUpload}>
            Try again
          </Button>
          <span className="text-sm text-gray-500">Resumes from where it stopped.</span>
        </div>
      ) : null}

      {error ? (
        <p className="text-sm text-red-600" role="alert">
          {error}
        </p>
      ) : null}
    </div>
  );
}
