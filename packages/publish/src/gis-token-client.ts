/**
 * Google Identity Services (GIS) token-client wrapper — the one PKCE-adjacent OAuth
 * implementation for the whole product (AGENTS.md §5). Structurally complete, but cannot be
 * exercised end-to-end until a real Google Cloud OAuth client id exists (manual maintainer
 * step, see the plan's "manual follow-up"). The wire-protocol logic it feeds,
 * GoogleDriveProvider, is unit-tested independently in packages/storage.
 */

const GIS_SCRIPT_SRC = "https://accounts.google.com/gsi/client";
export const DRIVE_FILE_SCOPE = "https://www.googleapis.com/auth/drive.file";

interface GisTokenResponse {
  access_token?: string;
  error?: string;
}

interface GisTokenClient {
  requestAccessToken: (overrideConfig?: { prompt?: string }) => void;
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
    script.onerror = () => reject(new Error("Failed to load Google Identity Services script"));
    document.head.appendChild(script);
  });
  return scriptLoadPromise;
}

export interface CreateGisTokenGetterOptions {
  clientId: string;
  scope?: string;
}

/**
 * Returns a getAccessToken() callback matching GoogleDriveProviderOptions. GIS access tokens
 * last ~1 hour with no refresh token (AGENTS.md §6), so every call re-requests a token; GIS
 * reuses the existing session silently (no visible prompt) when one is still active.
 *
 * initTokenClient's callback is fixed at creation time, so overlapping requestAccessToken()
 * calls are routed to their caller via a FIFO queue rather than a per-call callback override.
 */
export function createGisTokenGetter(options: CreateGisTokenGetterOptions): () => Promise<string> {
  const { clientId, scope = DRIVE_FILE_SCOPE } = options;
  let tokenClient: GisTokenClient | null = null;
  const pending: Array<{ resolve: (token: string) => void; reject: (err: Error) => void }> = [];

  async function ensureClient(): Promise<GisTokenClient> {
    await loadGisScript();
    if (!window.google) {
      throw new Error("Google Identity Services script did not initialize window.google");
    }
    if (!tokenClient) {
      tokenClient = window.google.accounts.oauth2.initTokenClient({
        client_id: clientId,
        scope,
        callback: (response) => {
          const next = pending.shift();
          if (!next) {
            return;
          }
          if (response.error || !response.access_token) {
            next.reject(new Error(response.error ?? "Google Identity Services returned no access_token"));
            return;
          }
          next.resolve(response.access_token);
        },
      });
    }
    return tokenClient;
  }

  return async function getAccessToken(): Promise<string> {
    const client = await ensureClient();
    return new Promise<string>((resolve, reject) => {
      pending.push({ resolve, reject });
      client.requestAccessToken();
    });
  };
}
