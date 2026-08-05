/**
 * Screen capture source kinds accepted by getDisplayMedia's constraints.
 */
export type CaptureSourceKind = "screen" | "window" | "tab";

export type ContainerFormat = "mp4" | "webm";

export interface ContainerCandidate {
  /** MediaRecorder mimeType string, gated by MediaRecorder.isTypeSupported() at call time. */
  readonly mimeType: string;
  readonly container: ContainerFormat;
}

/**
 * First match wins. MP4-first is conditional on Drive's preview player verifying
 * fragmented-MP4 playback/seek (ADR-003 §3.3) — do not reorder without re-verifying.
 */
export const CONTAINER_CANDIDATES: readonly ContainerCandidate[] = [
  { mimeType: "video/mp4;codecs=avc1.42E01E,mp4a.40.2", container: "mp4" },
  { mimeType: "video/webm;codecs=vp9,opus", container: "webm" },
  { mimeType: "video/webm;codecs=vp8,opus", container: "webm" },
  { mimeType: "video/webm", container: "webm" },
];
