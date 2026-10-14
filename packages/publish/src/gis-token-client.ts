/**
 * Google Identity Services (GIS) token-client wrapper — the one OAuth implementation for the
 * whole product (AGENTS.md §5). This is the GIS *token model*: a public client with no client
 * secret and no refresh token, not authorization-code + PKCE (Google's token endpoint wants a
 * client secret for "Web application" clients, which a static page cannot hold). Structurally
 * complete, but cannot be exercised end-to-end until a real Google Cloud OAuth client id
 * exists (docs/oauth-setup.md). The wire-protocol logic it feeds, GoogleDriveProvider, is
 * unit-tested independently in packages/storage.
 */

import type { AccessTokenRequest } from "@vrecnu/storage";

const GIS_SCRIPT_SRC = "https://accounts.google.com/gsi/client";
export const DRIVE_FILE_SCOPE = "https://www.googleapis.com/auth/drive.file";

/**
 * Re-request this long before Google's stated expiry, so a token never lapses mid-chunk
 * (capped at half the lifetime for unusually short tokens).
 */
const EXPIRY_MARGIN_MS = 5 * 60 * 1000;
/** GIS documents ~1 hour; only used if a response omits `expires_in`. */
const DEFAULT_LIFETIME_S = 3600;

interface GisTokenResponse {
  access_token?: string;
  /** Lifetime in seconds. */
  expires_in?: number | string;
  /** Space-delimited scopes actually granted — granular consent can leave ours out. */
  scope?: string;
  error?: string;
}

/** Popup-level failures. GIS never calls `callback` for these, only `error_callback`. */
interface GisErrorResponse {
  type: string;
  message?: string;
}

interface GisTokenClient {
  requestAccessToken: (overrideConfig?: { prompt?: string }) => void;
}

/** The one popup request currently open, shared by everyone waiting on a token. */
interface Pending {
  promise: Promise<string>;
  resolve: (token: string) => void;
  reject: (err: Error) => void;
}

declare global {
  interface Window {
    google?: {
      accounts: {
        oauth2: {
          initTokenClient: (config: {
            client_id: string;
            scope: string;
            callback: (response: GisTokenResponse) => void;
            error_callback?: (error: GisErrorResponse) => void;
          }) => GisTokenClient;
        };
      };
    };
  }
}

let scriptLoadPromise: Promise<void> | null = null;

export function loadGisScript(): Promise<void> {
  if (scriptLoadPromise) {
    return scriptLoadPromise;
  }
  scriptLoadPromise = new Promise((resolve, reject) => {
    if (document.querySelector(`script[src="${GIS_SCRIPT_SRC}"]`)) {
      resolve();
      return;
    }
    const script = document.createElement("script");
    script.src = GIS_SCRIPT_SRC;
    script.async = true;
    script.defer = true;
    script.onload = () => resolve();
    script.onerror = () => {
      // Forget the failure so "Try again" can reload the script instead of replaying it.
      scriptLoadPromise = null;
      script.remove();
      reject(new Error("Failed to load Google Identity Services script"));
    };
    document.head.appendChild(script);
  });
  return scriptLoadPromise;
}

export interface CreateGisTokenGetterOptions {
  clientId: string;
  scope?: string;
  /** Injected so tests can move the clock past a token's expiry. */
  now?: () => number;
}

function describePopupError(error: GisErrorResponse): string {
  switch (error.type) {
    case "popup_failed_to_open":
      return "The browser blocked the Google sign-in window. Allow pop-ups for this page and try again — an interrupted upload resumes where it stopped.";
    case "popup_closed":
      return "The Google sign-in window was closed before sign-in finished. Try again to continue.";
    default:
      return `Google sign-in failed${error.message ? `: ${error.message}` : ""}.`;
  }
}

/**
 * Returns a getAccessToken() callback matching GoogleDriveProviderOptions. The provider asks
 * for a token before every request (one per 8 MiB chunk), so this caches the token until
 * shortly before it expires — each GIS request is a popup, and a popup that does not come
 * from a user gesture is fair game for the popup blocker. That makes the first request, made
 * from the "Connect Google Drive" click, the one that matters; later refreshes ask with
 * `prompt: ""` so Google only shows consent again if the grant was revoked.
 *
 * Concurrent callers share one in-flight request: initTokenClient's callbacks are fixed at
 * creation time, so two overlapping popups could not be told apart anyway.
 */
export function createGisTokenGetter(
  options: CreateGisTokenGetterOptions,
): (request?: AccessTokenRequest) => Promise<string> {
  const { clientId, scope = DRIVE_FILE_SCOPE, now = Date.now } = options;
  let tokenClient: GisTokenClient | null = null;
  let cached: { token: string; refreshAt: number } | null = null;
  let inflight: Pending | null = null;
  let granted = false;

  function settle(outcome: { token: string } | { error: Error }): void {
    const request = inflight;
    inflight = null;
    if (!request) {
      // A late answer to a request nobody is waiting on any more.
      return;
    }
    if ("error" in outcome) {
      request.reject(outcome.error);
    } else {
      request.resolve(outcome.token);
    }
  }

  function handleResponse(response: GisTokenResponse): void {
    if (response.error || !response.access_token) {
      settle({ error: new Error(response.error ?? "Google Identity Services returned no access_token") });
      return;
    }
    if (response.scope !== undefined) {
      const grantedScopes = response.scope.split(" ");
      if (!scope.split(" ").every((wanted) => grantedScopes.includes(wanted))) {
        settle({
          error: new Error(
            "Google Drive access wasn't granted. Connect again and allow vrecnu to manage the files it creates.",
          ),
        });
        return;
      }
    }
    const lifetimeS = Number(response.expires_in);
    const lifetimeMs = (Number.isFinite(lifetimeS) && lifetimeS > 0 ? lifetimeS : DEFAULT_LIFETIME_S) * 1000;
    // Never let the margin swallow a short lifetime whole, or every call would be a popup.
    const marginMs = Math.min(EXPIRY_MARGIN_MS, lifetimeMs / 2);
    cached = { token: response.access_token, refreshAt: now() + lifetimeMs - marginMs };
    granted = true;
    settle({ token: response.access_token });
  }

  async function ensureClient(): Promise<GisTokenClient> {
    await loadGisScript();
    if (!window.google) {
      throw new Error("Google Identity Services script did not initialize window.google");
    }
    if (!tokenClient) {
      tokenClient = window.google.accounts.oauth2.initTokenClient({
        client_id: clientId,
        scope,
        callback: handleResponse,
        // Without this, closing or blocking the popup left the returned promise pending
        // forever and the upload silently stalled.
        error_callback: (error) => settle({ error: new Error(describePopupError(error)) }),
      });
    }
    return tokenClient;
  }

  return async function getAccessToken(request: AccessTokenRequest = {}): Promise<string> {
    if (request.forceRefresh) {
      cached = null;
    }
    if (cached && now() < cached.refreshAt) {
      return cached.token;
    }
    if (!inflight) {
      const client = await ensureClient();
      // Re-checked after the await: another caller may have opened the popup meanwhile.
      if (!inflight) {
        inflight = createPending();
        client.requestAccessToken(granted ? { prompt: "" } : undefined);
      }
    }
    return inflight.promise;
  };
}

function createPending(): Pending {
  let resolve!: (token: string) => void;
  let reject!: (err: Error) => void;
  const promise = new Promise<string>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}
