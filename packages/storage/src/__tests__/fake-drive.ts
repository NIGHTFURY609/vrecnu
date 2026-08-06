/**
 * A fake Drive resumable-upload endpoint that actually implements the protocol: session
 * creation, `Content-Range` accounting, 308 + `Range` responses, the `bytes * /total` status
 * probe, expiry, and partial-chunk acceptance.
 *
 * This exists because vrecnu has no OAuth client yet, so the real round-trip cannot be run.
 * Mocking individual `fetch` calls would only prove the code calls fetch; modelling the
 * protocol proves the *resume logic* is right — that an interrupted upload continues from
 * where the server actually got to rather than where the client assumed.
 */

export interface FakeDriveOptions {
  /** Fail the Nth PUT (1-based) with this status before any bytes are accepted. */
  failChunkAttempts?: { attempt: number; status: number }[];
  /** Throw a network error on the Nth PUT (1-based). */
  networkErrorAttempts?: number[];
  /** Accept only this many bytes of each chunk, forcing the client to honour Range. */
  acceptOnlyBytes?: number;
  /** Respond 404 to everything after the session is created, simulating expiry. */
  expireAfterAttempts?: number;
}

export class FakeDrive {
  received = 0;
  totalSize = 0;
  putAttempts = 0;
  probeCount = 0;
  sessionsCreated = 0;
  lastCreateBody: Record<string, unknown> | null = null;
  private readonly sessionUri = "https://upload.example/session-fake";

  constructor(private readonly options: FakeDriveOptions = {}) {}

  readonly fetch = async (input: RequestInfo | URL, init: RequestInit = {}): Promise<Response> => {
    const url = String(input);

    if (url.includes("uploadType=resumable")) {
      this.sessionsCreated++;
      this.received = 0;
      this.lastCreateBody = JSON.parse(String(init.body)) as Record<string, unknown>;
      this.totalSize = Number(
        (init.headers as Record<string, string> | undefined)?.["X-Upload-Content-Length"] ?? 0,
      );
      return new Response(null, { status: 200, headers: { Location: this.sessionUri } });
    }

    if (url === this.sessionUri) {
      const headers = init.headers as Record<string, string>;
      const contentRange = headers["Content-Range"] ?? "";

      // Status probe: `bytes * /total`, no body.
      if (contentRange.startsWith("bytes */")) {
        this.probeCount++;
        if (this.received >= this.totalSize && this.totalSize > 0) {
          return jsonResponse({ id: "file-complete" }, 200);
        }
        return this.partialResponse();
      }

      this.putAttempts++;

      if (this.options.expireAfterAttempts !== undefined && this.putAttempts > this.options.expireAfterAttempts) {
        return new Response(null, { status: 404 });
      }
      if (this.options.networkErrorAttempts?.includes(this.putAttempts)) {
        throw new TypeError("Failed to fetch");
      }
      const forced = this.options.failChunkAttempts?.find((f) => f.attempt === this.putAttempts);
      if (forced) {
        return new Response(null, { status: forced.status });
      }

      const match = /bytes (\d+)-(\d+)\/(\d+)/.exec(contentRange);
      if (!match?.[1] || !match[2] || !match[3]) {
        return new Response(null, { status: 400 });
      }
      const start = Number(match[1]);
      const end = Number(match[2]);
      const total = Number(match[3]);
      this.totalSize = total;

      // A client that resumes from the wrong offset must be caught, not silently tolerated.
      if (start !== this.received) {
        return new Response(`expected offset ${this.received}, got ${start}`, { status: 400 });
      }

      const offered = end - start + 1;
      const accepted = this.options.acceptOnlyBytes ? Math.min(this.options.acceptOnlyBytes, offered) : offered;
      this.received = start + accepted;

      if (this.received >= total) {
        return jsonResponse({ id: "file-complete" }, 200);
      }
      return this.partialResponse();
    }

    if (url.includes("/permissions")) {
      return jsonResponse({ id: "perm-1" }, 200);
    }
    if (url.includes("fields=webViewLink")) {
      return jsonResponse({ webViewLink: "https://drive.google.com/file/d/file-complete/view" }, 200);
    }
    return new Response(null, { status: 404 });
  };

  get uri(): string {
    return this.sessionUri;
  }

  private partialResponse(): Response {
    if (this.received === 0) {
      // Drive omits Range entirely when it holds nothing.
      return new Response(null, { status: 308 });
    }
    return new Response(null, {
      status: 308,
      headers: { Range: `bytes=0-${this.received - 1}` },
    });
  }
}

function jsonResponse(body: unknown, status: number): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  });
}
