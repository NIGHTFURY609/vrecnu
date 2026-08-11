/**
 * Storage headroom checks (AGENTS.md §6, ADR-003 amendment — "mandatory rather than optional").
 *
 * Two distinct jobs, often confused:
 *  - `persist()` asks the browser to stop evicting our OPFS data under storage pressure.
 *    Without it, a long recording can be silently deleted mid-session.
 *  - `estimate()` tells us whether the disk can hold what the user is about to record.
 *    Finding out after 40 minutes that the quota ran out is the failure this prevents.
 */

/**
 * ~625 KB/s is the 1080p30 figure AGENTS.md §6 uses as its reference point
 * ("~625 KB/s against storage that sustains hundreds of MB/s").
 */
export const ESTIMATED_BYTES_PER_SECOND = 625 * 1024;

/** Headroom is judged against twice the projection — a recording should never end at the cliff. */
const HEADROOM_SAFETY_FACTOR = 2;

/** What we assume a session might run to when the user hasn't said. */
const ASSUMED_SESSION_SECONDS = 30 * 60;

export type PersistOutcome = "granted" | "denied" | "unsupported";

/**
 * Requests durable storage. Chromium generally grants this silently for installed/engaged
 * origins and denies it otherwise; a denial is worth surfacing but is not fatal.
 */
export async function requestPersistentStorage(): Promise<PersistOutcome> {
  if (!navigator.storage?.persist) {
    return "unsupported";
  }
  try {
    if (await navigator.storage.persisted()) {
      return "granted";
    }
    return (await navigator.storage.persist()) ? "granted" : "denied";
  } catch {
    return "unsupported";
  }
}

export interface StorageHeadroom {
  readonly supported: boolean;
  readonly quotaBytes: number | null;
  readonly usageBytes: number | null;
  readonly availableBytes: number | null;
  /** Bytes we expect the planned session to need, before the safety factor. */
  readonly projectedBytes: number;
  /** True when available space is under `projectedBytes * 2`. */
  readonly low: boolean;
  /** Roughly how long we could still record, in seconds. `null` when unknown. */
  readonly recordableSeconds: number | null;
}

export async function checkStorageHeadroom(
  plannedSeconds: number = ASSUMED_SESSION_SECONDS,
): Promise<StorageHeadroom> {
  const projectedBytes = plannedSeconds * ESTIMATED_BYTES_PER_SECOND;

  if (!navigator.storage?.estimate) {
    return {
      supported: false,
      quotaBytes: null,
      usageBytes: null,
      availableBytes: null,
      projectedBytes,
      low: false,
      recordableSeconds: null,
    };
  }

  try {
    const { quota, usage } = await navigator.storage.estimate();
    if (typeof quota !== "number" || typeof usage !== "number") {
      return {
        supported: false,
        quotaBytes: quota ?? null,
        usageBytes: usage ?? null,
        availableBytes: null,
        projectedBytes,
        low: false,
        recordableSeconds: null,
      };
    }
    const availableBytes = Math.max(0, quota - usage);
    return {
      supported: true,
      quotaBytes: quota,
      usageBytes: usage,
      availableBytes,
      projectedBytes,
      low: availableBytes < projectedBytes * HEADROOM_SAFETY_FACTOR,
      recordableSeconds: Math.floor(availableBytes / ESTIMATED_BYTES_PER_SECOND),
    };
  } catch {
    return {
      supported: false,
      quotaBytes: null,
      usageBytes: null,
      availableBytes: null,
      projectedBytes,
      low: false,
      recordableSeconds: null,
    };
  }
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) {
    return `${bytes} B`;
  }
  const units = ["KB", "MB", "GB", "TB"];
  let value = bytes / 1024;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit++;
  }
  // One decimal, but drop a bare ".0" so round numbers read as "2 GB", not "2.0 GB".
  return `${value.toFixed(1).replace(/\.0$/, "")} ${units[unit]}`;
}

export function formatDurationApprox(totalSeconds: number): string {
  if (totalSeconds < 60) {
    return `${totalSeconds}s`;
  }
  const minutes = Math.floor(totalSeconds / 60);
  if (minutes < 60) {
    return `${minutes} min`;
  }
  const hours = Math.floor(minutes / 60);
  const remainder = minutes % 60;
  return remainder === 0 ? `${hours} hr` : `${hours} hr ${remainder} min`;
}
