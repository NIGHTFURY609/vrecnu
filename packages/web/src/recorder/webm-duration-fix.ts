import fixWebmDuration from "fix-webm-duration";

/**
 * Patches the missing Info/Duration element MediaRecorder's WebM output omits (ADR-003 §3.2)
 * — a metadata patch, not a transcode. No-op if the blob already carries a valid duration.
 */
export async function patchWebmDuration(blob: Blob, durationMs: number): Promise<Blob> {
  return fixWebmDuration(blob, durationMs, { logger: false });
}
