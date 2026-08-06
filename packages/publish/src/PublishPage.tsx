import { useEffect, useState, type JSX } from "react";
import { probeRangeSourceSize, rangeByteSource, type ByteSource } from "@vrecnu/storage";
import { UploadPanel } from "./UploadPanel";
import { CredentialsPanel } from "./CredentialsPanel";

/**
 * What the desktop app's loopback server reports about the recording awaiting publication.
 * Served from `GET /api/session?t=<token>` on the same origin as this page (ADR-002).
 */
interface LoopbackSession {
  fileName: string;
  mimeType: string;
  sizeBytes: number;
  durationMs?: number;
  durationValid?: boolean;
}

type PageState =
  | { kind: "loading" }
  | { kind: "ready"; session: LoopbackSession; source: ByteSource; token: string }
  | { kind: "error"; message: string };

/** Best-effort notification back to the Rust side; never blocks the user's flow. */
async function report(path: string, token: string, body: Record<string, unknown>): Promise<void> {
  try {
    await fetch(path, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ token, ...body }),
    });
  } catch {
    // The desktop app also notices the window closing; a failed report is not fatal.
  }
}

/**
 * The standalone publish page — the one artifact Phase 1 and Phase 2 genuinely share
 * (ADR-002). The desktop app bundles this build into its exe and serves it from
 * `http://127.0.0.1:<port>`, then serves the recording from that *same* origin, so the fetch
 * below is same-origin by construction: no CORS grant, and the capability token never has to
 * survive a cross-origin request.
 *
 * The recording is read through a ranged `ByteSource`, never `fetch(url).then(r => r.blob())`
 * — a 500 MB Blob in the web view would reintroduce exactly the memory failure ADR-003 exists
 * to prevent.
 */
export function PublishPage(): JSX.Element {
  const [state, setState] = useState<PageState>({ kind: "loading" });
  const [credentialsNonce, setCredentialsNonce] = useState(0);

  useEffect(() => {
    let cancelled = false;

    void (async () => {
      const token = new URLSearchParams(window.location.search).get("t");
      if (!token) {
        if (!cancelled) {
          setState({ kind: "error", message: "This page was opened without a capability token." });
        }
        return;
      }

      try {
        const response = await fetch(`/api/session?t=${encodeURIComponent(token)}`);
        if (!response.ok) {
          throw new Error(`The recording is no longer available (${response.status}).`);
        }
        const session = (await response.json()) as LoopbackSession;
        const recordingUrl = `/r/${encodeURIComponent(token)}`;
        // Trust the server's Content-Length over the session metadata: it is what the range
        // requests will actually be served against.
        const size = await probeRangeSourceSize(recordingUrl).catch(() => session.sizeBytes);
        if (cancelled) {
          return;
        }
        setState({
          kind: "ready",
          session,
          token,
          source: rangeByteSource(recordingUrl, { size }),
        });
      } catch (err) {
        if (!cancelled) {
          setState({ kind: "error", message: err instanceof Error ? err.message : "Could not load the recording." });
        }
      }
    })();

    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <main className="mx-auto flex min-h-screen max-w-2xl flex-col items-center gap-6 p-8">
      <h1 className="text-2xl font-semibold">Publish to Google Drive</h1>

      {state.kind === "loading" ? (
        <p className="text-sm text-gray-500" role="status">
          Loading your recording…
        </p>
      ) : null}

      {state.kind === "error" ? (
        <p className="text-sm text-red-600" role="alert">
          {state.message}
        </p>
      ) : null}

      {state.kind === "ready" ? (
        <>
          <p className="text-sm text-gray-600">
            {state.session.fileName} · {(state.session.sizeBytes / (1024 * 1024)).toFixed(1)} MB
          </p>

          <CredentialsPanel
            buildTimeClientId={import.meta.env.VITE_GOOGLE_CLIENT_ID}
            onChanged={() => setCredentialsNonce((n) => n + 1)}
            showDesktopOrigins
          />

          <UploadPanel
            key={credentialsNonce}
            source={state.source}
            fileName={state.session.fileName}
            mimeType={state.session.mimeType}
            durationWarning={
              state.session.durationValid === false
                ? "This recording has no duration recorded, so it may not seek correctly in Drive's preview player."
                : null
            }
            onProgress={(fraction) =>
              void report("/api/progress", state.token, {
                bytesSent: Math.round(fraction * state.source.size),
                total: state.source.size,
              })
            }
            onComplete={({ fileId, webViewLink }) =>
              // The share link goes back over the loopback server, not Tauri IPC. Keeping it
              // HTTP means this page has no shell-specific branch and works unchanged in a
              // plain browser (ADR-002 / ADR-006).
              void report("/api/complete", state.token, { fileId, webViewLink })
            }
            onFailed={(message) => void report("/api/error", state.token, { message })}
          />
        </>
      ) : null}
    </main>
  );
}
