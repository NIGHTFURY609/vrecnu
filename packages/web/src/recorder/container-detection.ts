import { CONTAINER_CANDIDATES, type ContainerCandidate } from "@vrecnu/core";

export type SelectedContainer = ContainerCandidate;

/**
 * First MediaRecorder.isTypeSupported() match wins (ADR-003 §3.3's detection ladder).
 * MP4-first is conditional on Drive's preview player verifying fragmented-MP4 seek — that
 * verification is a manual follow-up, not something this spike can prove on its own.
 */
export function selectContainer(): SelectedContainer {
  const match = CONTAINER_CANDIDATES.find((candidate) => MediaRecorder.isTypeSupported(candidate.mimeType));
  if (!match) {
    throw new Error("No supported MediaRecorder mimeType found among CONTAINER_CANDIDATES");
  }
  return match;
}
