import { blobByteSource, type ByteSource } from "./byte-source";
import {
  UploadError,
  type ResumableSession,
  type StorageProvider,
  type UploadMetadata,
  type UploadOptions,
} from "./storage-provider";

const DRIVE_UPLOAD_ENDPOINT = "https://www.googleapis.com/upload/drive/v3/files";
const DRIVE_API_ENDPOINT = "https://www.googleapis.com/drive/v3/files";
const FOLDER_MIME_TYPE = "application/vnd.google-apps.folder";

/** Must be a multiple of 256 KiB per the Drive resumable-upload spec. */
const UPLOAD_CHUNK_SIZE = 8 * 1024 * 1024;

const DEFAULT_MAX_ATTEMPTS = 6;
const BASE_BACKOFF_MS = 500;
const MAX_BACKOFF_MS = 20_000;

/** Transient by Google's own definition — everything else is a real failure. */
function isRetryableStatus(status: number): boolean {
  return status === 408 || status === 429 || (status >= 500 && status < 600);
}

function defaultSleep(ms: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, ms));
}

export interface AccessTokenRequest {
  /** Drive rejected the last token (401): skip any cache and get a fresh one. */
  forceRefresh?: boolean;
}

/**
 * No token could be had at all — sign-in closed, blocked or denied. Kept distinct so the
 * retry paths below never mistake it for a dropped connection: retrying would open another
 * popup outside a user gesture and bury the reason the user needs to see.
 */
class TokenUnavailableError extends Error {}

/** A request whose headers can be extended with Authorization. */
type DriveRequestInit = Omit<RequestInit, "headers"> & { headers?: Record<string, string> };

export interface GoogleDriveProviderOptions {
  /**
   * GIS access tokens last ~1 hour with no refresh token (AGENTS.md §6). A large upload on a
   * slow connection can outlive one, so every request asks this callback for a token rather
   * than the provider holding a fixed string. The callback is expected to cache; on a 401 the
   * provider asks once more with `forceRefresh` before giving up.
   */
  getAccessToken: (request?: AccessTokenRequest) => Promise<string>;
  maxAttempts?: number;
  /** Injected so tests do not actually wait out the backoff. */
  sleep?: (ms: number) => Promise<void>;
  fetchImpl?: typeof fetch;
}

/**
 * Talks to Drive REST v3 using only drive.file-scope-compatible endpoints: create (this
 * app's own upload), permissions.create, and a metadata read of a file this app created.
 * Never lists or searches existing files (AGENTS.md §5).
 *
 * The upload is genuinely resumable, not merely chunked. Three things make that true, and
 * all three were missing before: the server is asked where it actually got to (rather than
 * assuming the last chunk landed), the `Range` header on a 308 is honoured, and transient
 * failures are retried with backoff instead of destroying the whole upload. AGENTS.md §5
 * calls a non-resuming upload "a defect, not an edge case".
 */
export class GoogleDriveProvider implements StorageProvider {
  private connected = false;
  private readonly maxAttempts: number;
  private readonly sleep: (ms: number) => Promise<void>;
  private readonly fetchImpl: typeof fetch;

  constructor(private readonly options: GoogleDriveProviderOptions) {
    this.maxAttempts = options.maxAttempts ?? DEFAULT_MAX_ATTEMPTS;
    this.sleep = options.sleep ?? defaultSleep;
    this.fetchImpl = options.fetchImpl ?? fetch;
  }

  isConnected(): boolean {
    return this.connected;
  }

  async connect(): Promise<void> {
    await this.options.getAccessToken();
    this.connected = true;
  }

  async disconnect(): Promise<void> {
    this.connected = false;
  }

  async createFolder(name: string): Promise<string> {
    const response = await this.authorizedFetch(DRIVE_API_ENDPOINT, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ name, mimeType: FOLDER_MIME_TYPE }),
    });
    if (!response.ok) {
      throw new UploadError(`Drive folder create failed: ${response.status} ${response.statusText}`, false);
    }
    const { id } = (await response.json()) as { id: string };
    return id;
  }

  async uploadResumable(
    source: Blob | ByteSource,
    metadata: UploadMetadata,
    options: UploadOptions = {},
  ): Promise<{ fileId: string }> {
    const file = source instanceof Blob ? blobByteSource(source) : source;
    let sessionUri = options.resumeSessionUri ?? null;
    let startOffset = 0;

    if (sessionUri) {
      const probed = await this.probeOffset(sessionUri, file.size, options);
      if (probed.kind === "complete") {
        options.onSessionEnd?.(sessionUri);
        return { fileId: probed.fileId };
      }
      if (probed.kind === "expired") {
        options.onSessionEnd?.(sessionUri);
        sessionUri = null;
      } else {
        startOffset = probed.offset;
      }
    }

    if (!sessionUri) {
      sessionUri = await this.initResumableSession(file, metadata);
      const session: ResumableSession = {
        uri: sessionUri,
        fileName: metadata.name,
        fileSize: file.size,
        mimeType: metadata.mimeType,
        createdAt: Date.now(),
      };
      options.onSessionStart?.(session);
    }

    const fileId = await this.putChunks(sessionUri, file, startOffset, options);
    options.onSessionEnd?.(sessionUri);
    return { fileId };
  }

  async createShareLink(fileId: string): Promise<string> {
    const permResponse = await this.authorizedFetch(`${DRIVE_API_ENDPOINT}/${fileId}/permissions`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify({ role: "reader", type: "anyone" }),
    });
    if (!permResponse.ok) {
      throw new UploadError(
        `Drive permissions.create failed: ${permResponse.status} ${permResponse.statusText}`,
        isRetryableStatus(permResponse.status),
      );
    }

    const metaResponse = await this.authorizedFetch(`${DRIVE_API_ENDPOINT}/${fileId}?fields=webViewLink`);
    if (!metaResponse.ok) {
      throw new UploadError(
        `Drive file metadata fetch failed: ${metaResponse.status} ${metaResponse.statusText}`,
        isRetryableStatus(metaResponse.status),
      );
    }
    const { webViewLink } = (await metaResponse.json()) as { webViewLink: string };
    return webViewLink;
  }

  private async initResumableSession(file: ByteSource, metadata: UploadMetadata): Promise<string> {
    const body: Record<string, unknown> = { name: metadata.name };
    if (metadata.folderId) {
      body.parents = [metadata.folderId];
    }
    const response = await this.authorizedFetch(`${DRIVE_UPLOAD_ENDPOINT}?uploadType=resumable`, {
      method: "POST",
      headers: {
        "Content-Type": "application/json; charset=UTF-8",
        "X-Upload-Content-Type": metadata.mimeType,
        "X-Upload-Content-Length": String(file.size),
      },
      body: JSON.stringify(body),
    });
    if (!response.ok) {
      throw new UploadError(
        `Drive resumable session init failed: ${response.status} ${response.statusText}`,
        isRetryableStatus(response.status),
      );
    }
    const location = response.headers.get("Location");
    if (!location) {
      throw new UploadError("Drive resumable session init did not return a Location header", false);
    }
    return location;
  }

  /**
   * Asks the server how much it actually holds, via the zero-length `bytes * /total` form.
   * This is the step that turns "chunked" into "resumable" — without it, a retry has to guess
   * its offset, and a guess that is wrong corrupts the file.
   */
  private async probeOffset(
    sessionUri: string,
    totalBytes: number,
    options: UploadOptions,
  ): Promise<{ kind: "partial"; offset: number } | { kind: "complete"; fileId: string } | { kind: "expired" }> {
    const response = await this.authorizedFetch(sessionUri, {
      method: "PUT",
      headers: { "Content-Range": `bytes */${totalBytes}` },
      ...(options.signal ? { signal: options.signal } : {}),
    });

    if (response.status === 200 || response.status === 201) {
      const { id } = (await response.json()) as { id: string };
      return { kind: "complete", fileId: id };
    }
    if (response.status === 404 || response.status === 410) {
      return { kind: "expired" };
    }
    if (response.status === 308) {
      return { kind: "partial", offset: parseRangeEnd(response.headers.get("Range")) };
    }
    throw new UploadError(
      `Drive upload status probe failed: ${response.status} ${response.statusText}`,
      isRetryableStatus(response.status),
    );
  }

  private async putChunks(
    sessionUri: string,
    file: ByteSource,
    startOffset: number,
    options: UploadOptions,
  ): Promise<string> {
    const totalBytes = file.size;
    let offset = startOffset;
    let attempt = 0;
    let needsFreshToken = false;
    let reauthorised = false;

    options.onProgress?.({ bytesUploaded: offset, totalBytes });

    while (offset < totalBytes) {
      if (options.signal?.aborted) {
        throw new UploadError("Upload cancelled", true);
      }

      const end = Math.min(offset + UPLOAD_CHUNK_SIZE, totalBytes);
      const chunk = await file.slice(offset, end);
      // Asked per chunk, but the getter caches, so this only reaches Google near expiry or
      // after a 401. The resumable session URI, not the token, is what survives re-auth.
      // Kept outside the network try/catch: a blocked sign-in must fail now, not be retried
      // as if it were a dropped connection.
      const token = await this.accessToken(needsFreshToken ? { forceRefresh: true } : undefined);
      needsFreshToken = false;

      let response: Response;
      try {
        response = await this.fetchImpl(sessionUri, {
          method: "PUT",
          headers: {
            Authorization: `Bearer ${token}`,
            "Content-Range": `bytes ${offset}-${end - 1}/${totalBytes}`,
          },
          body: chunk,
          ...(options.signal ? { signal: options.signal } : {}),
        });
      } catch (err) {
        // A dropped connection is the single most likely thing to happen during a long
        // upload, and it used to lose the entire recording. Back off, re-probe, continue.
        if (options.signal?.aborted) {
          throw new UploadError("Upload cancelled", true);
        }
        attempt = await this.backoff(attempt, err instanceof Error ? err.message : "Network error");
        offset = await this.reprobe(sessionUri, totalBytes, options, offset);
        // The fresh token was never tested, so a later 401 still deserves its one re-auth;
        // `backoff` bounds this path on its own.
        reauthorised = false;
        continue;
      }

      if (response.status === 401 && !reauthorised) {
        // The token was revoked or lapsed early. Nothing was stored, so re-authorise once
        // and resend this chunk from the same offset; a second 401 falls through and fails.
        reauthorised = true;
        needsFreshToken = true;
        continue;
      }
      reauthorised = false;

      if (response.status === 308) {
        // Trust the server's Range over our own arithmetic: a partially accepted chunk
        // reports less than we sent, and assuming otherwise silently corrupts the upload.
        const acknowledged = parseRangeEnd(response.headers.get("Range"));
        offset = acknowledged > offset ? acknowledged : end;
        attempt = 0;
        options.onProgress?.({ bytesUploaded: offset, totalBytes });
        continue;
      }

      if (response.status === 200 || response.status === 201) {
        const { id } = (await response.json()) as { id: string };
        options.onProgress?.({ bytesUploaded: totalBytes, totalBytes });
        return id;
      }

      if (response.status === 404 || response.status === 410) {
        throw new UploadError("The upload session expired; start the upload again.", true, true);
      }

      if (isRetryableStatus(response.status)) {
        attempt = await this.backoff(attempt, `${response.status} ${response.statusText}`);
        offset = await this.reprobe(sessionUri, totalBytes, options, offset);
        continue;
      }

      throw new UploadError(`Drive chunk upload failed: ${response.status} ${response.statusText}`, false);
    }

    // Every byte is acknowledged but no completion body arrived — ask once more.
    const probed = await this.probeOffset(sessionUri, totalBytes, options);
    if (probed.kind === "complete") {
      options.onProgress?.({ bytesUploaded: totalBytes, totalBytes });
      return probed.fileId;
    }
    throw new UploadError("Drive resumable upload ended without a completion response", true);
  }

  /**
   * One request, re-authorised once on a 401. The token getter caches, and a cached token can
   * be revoked or expire early; failing outright on the first stale one would strand the
   * resumable session for no reason.
   */
  private async authorizedFetch(url: string, init: DriveRequestInit = {}): Promise<Response> {
    const send = (token: string): Promise<Response> =>
      this.fetchImpl(url, { ...init, headers: { ...init.headers, Authorization: `Bearer ${token}` } });
    const response = await send(await this.accessToken());
    if (response.status !== 401) {
      return response;
    }
    return send(await this.accessToken({ forceRefresh: true }));
  }

  private async accessToken(request?: AccessTokenRequest): Promise<string> {
    try {
      return await this.options.getAccessToken(request);
    } catch (err) {
      throw new TokenUnavailableError(err instanceof Error ? err.message : "Google sign-in failed.");
    }
  }

  /** Re-probes after a failure; falls back to the known offset if the probe itself fails. */
  private async reprobe(
    sessionUri: string,
    totalBytes: number,
    options: UploadOptions,
    fallback: number,
  ): Promise<number> {
    try {
      const probed = await this.probeOffset(sessionUri, totalBytes, options);
      return probed.kind === "partial" ? probed.offset : fallback;
    } catch (err) {
      if (err instanceof TokenUnavailableError) {
        throw err;
      }
      return fallback;
    }
  }

  private async backoff(attempt: number, reason: string): Promise<number> {
    const next = attempt + 1;
    if (next >= this.maxAttempts) {
      throw new UploadError(`Drive upload failed after ${this.maxAttempts} attempts: ${reason}`, true);
    }
    // Full jitter: retries from many clients must not re-synchronise onto the same instant.
    const ceiling = Math.min(MAX_BACKOFF_MS, BASE_BACKOFF_MS * 2 ** attempt);
    await this.sleep(Math.random() * ceiling);
    return next;
  }
}

/**
 * Drive answers a 308 with `Range: bytes=0-N`, where N is the last byte *stored*. The next
 * offset is therefore N+1. A missing header means nothing has been stored yet.
 */
export function parseRangeEnd(header: string | null): number {
  if (!header) {
    return 0;
  }
  const match = /bytes=(\d+)-(\d+)/.exec(header);
  if (!match?.[2]) {
    return 0;
  }
  return Number(match[2]) + 1;
}
