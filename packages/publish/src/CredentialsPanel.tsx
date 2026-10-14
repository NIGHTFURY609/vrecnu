import { useState, type JSX } from "react";
import { Button } from "@vrecnu/ui";
import {
  clearUserClientId,
  DESKTOP_LOOPBACK_ORIGINS,
  looksLikeClientId,
  resolveCredentials,
  saveUserClientId,
} from "./credentials";

export interface CredentialsPanelProps {
  // `| undefined` is explicit because the repo runs with `exactOptionalPropertyTypes`, and
  // `import.meta.env.VITE_GOOGLE_CLIENT_ID` is genuinely `string | undefined`.
  buildTimeClientId?: string | undefined;
  onChanged?: () => void;
  /** Show the loopback-origin list, which only matters for the desktop app (ADR-002). */
  showDesktopOrigins?: boolean;
}

/**
 * Bring-your-own-credentials (AGENTS.md §5). A user running their own Google Cloud project
 * pastes their client id here instead of trusting ours — the same posture rclone takes.
 *
 * No client secret is ever requested: the GIS token client is a public-client flow, and §5
 * forbids a secret in any shipped bundle.
 */
export function CredentialsPanel({
  buildTimeClientId,
  onChanged,
  showDesktopOrigins,
}: CredentialsPanelProps): JSX.Element {
  const [resolved, setResolved] = useState(() => resolveCredentials(buildTimeClientId));
  const [draft, setDraft] = useState("");
  const [expanded, setExpanded] = useState(() => resolved.source === "none");
  const [warning, setWarning] = useState<string | null>(null);

  const apply = (): void => {
    const value = draft.trim();
    if (!looksLikeClientId(value)) {
      setWarning("That doesn't look like a Google client ID (it should end in .apps.googleusercontent.com).");
      return;
    }
    saveUserClientId(value);
    setResolved(resolveCredentials(buildTimeClientId));
    setWarning(null);
    setDraft("");
    onChanged?.();
  };

  const reset = (): void => {
    clearUserClientId();
    setResolved(resolveCredentials(buildTimeClientId));
    onChanged?.();
  };

  return (
    <section className="w-full max-w-xl rounded-md border border-slate-200 p-3 text-sm">
      <div className="flex items-center justify-between gap-2">
        <h2 className="font-medium">Google Drive credentials</h2>
        <Button size="sm" variant="ghost" onClick={() => setExpanded((prev) => !prev)} aria-expanded={expanded}>
          {expanded ? "Hide" : "Change"}
        </Button>
      </div>

      <p className="mt-1 text-gray-600">
        {resolved.source === "user"
          ? "Using your own Google Cloud client."
          : resolved.source === "build"
            ? "Using this build's default client."
            : "No client ID configured — uploads run in demo mode."}
      </p>

      {expanded ? (
        <div className="mt-3 flex flex-col gap-2">
          <label className="flex flex-col gap-1">
            <span className="text-gray-700">OAuth client ID</span>
            <input
              type="text"
              value={draft}
              onChange={(event) => setDraft(event.target.value)}
              placeholder="1234567890-abc123.apps.googleusercontent.com"
              className="rounded-md border border-slate-300 px-2 py-1 font-mono text-xs"
            />
          </label>

          <p className="text-xs text-gray-600">
            Create an OAuth <strong>Web application</strong> client in Google Cloud Console with the{" "}
            <code className="font-mono">drive.file</code> scope only, and add this page&apos;s origin (
            <code className="font-mono">{window.location.origin}</code>) as an Authorized JavaScript Origin.
          </p>

          {showDesktopOrigins ? (
            <div className="text-xs text-gray-600">
              <p>The desktop app also needs all five of these registered as Authorized JavaScript Origins:</p>
              <ul className="mt-1 font-mono">
                {DESKTOP_LOOPBACK_ORIGINS.map((origin) => (
                  <li key={origin}>{origin}</li>
                ))}
              </ul>
            </div>
          ) : null}

          {warning ? (
            <p className="text-xs text-red-600" role="alert">
              {warning}
            </p>
          ) : null}

          <div className="flex gap-2">
            <Button size="sm" onClick={apply} disabled={draft.trim().length === 0}>
              Save
            </Button>
            {resolved.source === "user" ? (
              <Button size="sm" variant="outline" onClick={reset}>
                Use the default
              </Button>
            ) : null}
          </div>
        </div>
      ) : null}
    </section>
  );
}
