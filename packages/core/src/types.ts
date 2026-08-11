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
 * First match wins.
 *
 * WebM leads, MP4 is the fallback for browsers that cannot produce WebM (Safari). ADR-003
 * §3.3 made MP4-first *conditional* on verifying fragmented-MP4 playback and seek in Drive's
 * preview player, and its action item 2 is still unchecked — so shipping MP4 first would be
 * shipping on the assumption the ADR explicitly forbids.
 *
 * Two things follow from putting WebM first, both of which we can actually deliver today:
 * the duration fix is well-defined for WebM (see `webm-duration.ts`), and lossless Cluster
 * trim exists for WebM only (`webm-trim.ts`). Reordering requires verifying Drive preview
 * seek on a real fragmented-MP4 recording and recording the result in ADR-003.
 */
export const CONTAINER_CANDIDATES: readonly ContainerCandidate[] = [
  { mimeType: "video/webm;codecs=vp9,opus", container: "webm" },
  { mimeType: "video/webm;codecs=vp8,opus", container: "webm" },
  { mimeType: "video/webm", container: "webm" },
  { mimeType: "video/mp4;codecs=avc1.42E01E,mp4a.40.2", container: "mp4" },
];
