/**
 * Where the Google OAuth client id comes from.
 *
 * AGENTS.md §5 requires a bring-your-own-credentials path ("like rclone"). Previously the id
 * was read from a build-time env var only, so a self-hosting user had no way to supply their
 * own without rebuilding — which is not a BYO path at all.
 *
 * Resolution order: a user-supplied id (persisted locally) beats the build-time default, so
 * someone running their own Google Cloud project can override a hosted build.
 *
 * There is no client *secret* anywhere in this flow, by design — the GIS token client is a
 * public-client flow and AGENTS.md §5 forbids shipping a secret in any bundle.
 */

const STORAGE_KEY = "vrecnu.google.clientId";

export interface ResolvedCredentials {
  clientId: string | null;
  source: "user" | "build" | "none";
}

function readStored(): string | null {
  try {
    const value = window.localStorage.getItem(STORAGE_KEY);
    return value && value.trim().length > 0 ? value.trim() : null;
  } catch {
    return null;
  }
}

export function resolveCredentials(buildTimeClientId?: string): ResolvedCredentials {
  const stored = readStored();
  if (stored) {
    return { clientId: stored, source: "user" };
  }
  if (buildTimeClientId && buildTimeClientId.trim().length > 0) {
    return { clientId: buildTimeClientId.trim(), source: "build" };
  }
  return { clientId: null, source: "none" };
}

export function saveUserClientId(clientId: string): void {
  try {
    window.localStorage.setItem(STORAGE_KEY, clientId.trim());
  } catch {
    // Private mode. The id simply will not persist across reloads.
  }
}

export function clearUserClientId(): void {
  try {
    window.localStorage.removeItem(STORAGE_KEY);
  } catch {
    // Nothing actionable.
  }
}

/**
 * A Google OAuth client id looks like `<digits>-<hash>.apps.googleusercontent.com`. Checked
 * so an obviously wrong paste fails here rather than as an opaque Google error screen.
 */
export function looksLikeClientId(value: string): boolean {
  return /^[0-9]+-[a-z0-9]+\.apps\.googleusercontent\.com$/i.test(value.trim());
}

/**
 * The loopback origins the Phase 2 desktop app serves its publish page from (ADR-002).
 * A BYO user must register all five in their own Google Cloud project, so the setup UI can
 * show this list verbatim.
 */
export const DESKTOP_LOOPBACK_ORIGINS = [
  "http://127.0.0.1:47821",
  "http://127.0.0.1:47822",
  "http://127.0.0.1:47823",
  "http://127.0.0.1:47824",
  "http://127.0.0.1:47825",
] as const;
