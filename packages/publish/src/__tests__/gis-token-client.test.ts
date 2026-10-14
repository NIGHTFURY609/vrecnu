import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createGisTokenGetter, DRIVE_FILE_SCOPE } from "../gis-token-client";

interface FakeTokenClientConfig {
  callback: (response: Record<string, unknown>) => void;
  error_callback?: (error: { type: string; message?: string }) => void;
}

/**
 * Stands in for Google Identity Services. Each `requestAccessToken()` is one popup; the test
 * decides how Google answers it (grant, deny, popup closed or blocked).
 */
function installFakeGis() {
  let config: FakeTokenClientConfig | null = null;
  const requests: ({ prompt?: string } | undefined)[] = [];

  // The GIS <script> is already on the page, so loadGisScript() resolves without a network.
  vi.stubGlobal("document", { querySelector: () => ({}) });
  vi.stubGlobal("window", {
    google: {
      accounts: {
        oauth2: {
          initTokenClient: (c: FakeTokenClientConfig) => {
            config = c;
            return { requestAccessToken: (override?: { prompt?: string }) => void requests.push(override) };
          },
        },
      },
    },
  });

  const active = (): FakeTokenClientConfig => {
    if (!config) {
      throw new Error("initTokenClient was never called");
    }
    return config;
  };

  return {
    requests,
    grant: (token: string, extra: Record<string, unknown> = {}) =>
      active().callback({ access_token: token, expires_in: 3599, scope: DRIVE_FILE_SCOPE, ...extra }),
    deny: (error: string) => active().callback({ error }),
    popupError: (type: string) => active().error_callback?.({ type, message: type }),
  };
}

/** Lets the getter's `await ensureClient()` run so the popup request is actually issued. */
async function popupOpened(gis: { requests: unknown[] }, count: number): Promise<void> {
  await vi.waitFor(() => expect(gis.requests).toHaveLength(count));
}

describe("createGisTokenGetter", () => {
  let gis: ReturnType<typeof installFakeGis>;
  let clock = 0;
  const now = () => clock;

  beforeEach(() => {
    clock = 1_000_000;
    gis = installFakeGis();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("returns the cached token instead of opening a popup for every request", async () => {
    // The upload loop asks for a token per 8 MiB chunk; without a cache that is a popup per
    // chunk, and every one outside a user gesture is a candidate for the popup blocker.
    const getToken = createGisTokenGetter({ clientId: "id", now });
    const first = getToken();
    await popupOpened(gis, 1);
    gis.grant("tok-1");
    await expect(first).resolves.toBe("tok-1");

    clock += 30 * 60 * 1000;
    await expect(getToken()).resolves.toBe("tok-1");
    expect(gis.requests).toHaveLength(1);
  });

  it("re-requests shortly before expiry, without forcing the consent screen again", async () => {
    const getToken = createGisTokenGetter({ clientId: "id", now });
    const first = getToken();
    await popupOpened(gis, 1);
    gis.grant("tok-1", { expires_in: 3600 });
    await first;

    clock += 58 * 60 * 1000;
    const second = getToken();
    await popupOpened(gis, 2);
    expect(gis.requests[1]).toEqual({ prompt: "" });
    gis.grant("tok-2");
    await expect(second).resolves.toBe("tok-2");
  });

  it("still caches a short-lived token rather than re-requesting on every call", async () => {
    const getToken = createGisTokenGetter({ clientId: "id", now });
    const first = getToken();
    await popupOpened(gis, 1);
    gis.grant("tok-1", { expires_in: 240 });
    await first;

    clock += 60 * 1000;
    await expect(getToken()).resolves.toBe("tok-1");
    expect(gis.requests).toHaveLength(1);
  });

  it("skips the cache when the caller reports the token was rejected", async () => {
    const getToken = createGisTokenGetter({ clientId: "id", now });
    const first = getToken();
    await popupOpened(gis, 1);
    gis.grant("tok-1");
    await first;

    const refreshed = getToken({ forceRefresh: true });
    await popupOpened(gis, 2);
    gis.grant("tok-2");
    await expect(refreshed).resolves.toBe("tok-2");
  });

  it("shares one popup between concurrent callers", async () => {
    const getToken = createGisTokenGetter({ clientId: "id", now });
    const a = getToken();
    const b = getToken();
    await popupOpened(gis, 1);
    gis.grant("tok-1");
    await expect(Promise.all([a, b])).resolves.toEqual(["tok-1", "tok-1"]);
    expect(gis.requests).toHaveLength(1);
  });

  it("rejects instead of hanging when the sign-in popup is closed, and can be retried", async () => {
    const getToken = createGisTokenGetter({ clientId: "id", now });
    const first = getToken();
    await popupOpened(gis, 1);
    gis.popupError("popup_closed");
    await expect(first).rejects.toThrow(/closed/i);

    const retry = getToken();
    await popupOpened(gis, 2);
    gis.grant("tok-1");
    await expect(retry).resolves.toBe("tok-1");
  });

  it("rejects with an actionable message when the popup is blocked", async () => {
    const getToken = createGisTokenGetter({ clientId: "id", now });
    const first = getToken();
    await popupOpened(gis, 1);
    gis.popupError("popup_failed_to_open");
    await expect(first).rejects.toThrow(/blocked/i);
  });

  it("rejects a token that was granted without Drive access", async () => {
    // Granular consent lets the user untick the Drive permission; every upload call would
    // then fail with an opaque 403, so say what happened up front.
    const getToken = createGisTokenGetter({ clientId: "id", now });
    const first = getToken();
    await popupOpened(gis, 1);
    gis.grant("tok-1", { scope: "openid email" });
    await expect(first).rejects.toThrow(/Drive access/);

    // Nothing was cached, so the next request asks again.
    const retry = getToken();
    await popupOpened(gis, 2);
    gis.grant("tok-2");
    await expect(retry).resolves.toBe("tok-2");
  });

  it("surfaces Google's error code when the user denies access", async () => {
    const getToken = createGisTokenGetter({ clientId: "id", now });
    const first = getToken();
    await popupOpened(gis, 1);
    gis.deny("access_denied");
    await expect(first).rejects.toThrow(/access_denied/);
  });
});
