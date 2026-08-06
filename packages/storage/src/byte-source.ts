/**
 * A lazily-read sequence of bytes.
 *
 * This exists so the resumable upload loop is written **once** and works for both delivery
 * paths (ADR-002): in the PWA the bytes are an OPFS `File` and a slice is `file.slice()`; in
 * the Phase 2 desktop app the bytes live on disk behind the loopback server and a slice is a
 * ranged `fetch`. Both are lazy — neither materialises the recording.
 *
 * The alternative, `fetch(url).then(r => r.blob())`, would pull a multi-gigabyte recording
 * into the web view. That is precisely the memory failure ADR-003 was written to eliminate,
 * and it would be just as wrong on the desktop side as it is in the browser.
 */
export interface ByteSource {
  readonly size: number;
  /** `[start, end)`, matching `Blob.slice`. Must not read anything outside that window. */
  slice(start: number, end: number): Promise<Blob>;
}

/** The PWA path: an OPFS-backed `File` (or any Blob). Slicing a Blob does not read it. */
export function blobByteSource(blob: Blob): ByteSource {
  return {
    size: blob.size,
    slice: (start, end) => Promise.resolve(blob.slice(start, end)),
  };
}

export interface RangeByteSourceOptions {
  /** Total size, normally learned from a `HEAD` before the upload starts. */
  size: number;
  fetchImpl?: typeof fetch;
}

/**
 * The Phase 2 desktop path: bytes served by the loopback server on `127.0.0.1`
 * (ADR-002). Same-origin by construction, so no CORS and no cross-origin token handling —
 * the capability token is already in the URL.
 */
export function rangeByteSource(url: string, options: RangeByteSourceOptions): ByteSource {
  const doFetch = options.fetchImpl ?? fetch;
  return {
    size: options.size,
    slice: async (start, end) => {
      // HTTP Range is inclusive at both ends; ByteSource is half-open like Blob.slice.
      const response = await doFetch(url, { headers: { Range: `bytes=${start}-${end - 1}` } });
      if (response.status !== 206 && response.status !== 200) {
        throw new Error(`Range request failed: ${response.status} ${response.statusText}`);
      }
      return await response.blob();
    },
  };
}

/** Reads the total size without transferring the body. */
export async function probeRangeSourceSize(url: string, fetchImpl: typeof fetch = fetch): Promise<number> {
  const response = await fetchImpl(url, { method: "HEAD" });
  if (!response.ok) {
    throw new Error(`Could not read recording size: ${response.status} ${response.statusText}`);
  }
  const length = response.headers.get("Content-Length");
  if (!length) {
    throw new Error("Recording endpoint did not report a Content-Length");
  }
  return Number(length);
}
