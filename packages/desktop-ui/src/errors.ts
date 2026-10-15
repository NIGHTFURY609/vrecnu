import type { AppError } from "./engine";

/**
 * Commands reject with the engine's AppError. Anything else (an unknown command, a closed IPC
 * channel) is a Tauri-level failure: wrapped so the UI renders one shape, and never marked as
 * having lost a recording it knows nothing about.
 */
export function toAppError(err: unknown): AppError {
  if (isAppError(err)) return err;
  const detail = err instanceof Error ? err.message : String(err);
  return {
    code: "INTERNAL",
    message: "vrecnu could not reach its engine.",
    hint: "Close the window and open it again from the tray.",
    detail,
    recoverable: true,
    sessionLost: false,
  };
}

function isAppError(value: unknown): value is AppError {
  if (typeof value !== "object" || value === null) return false;
  const record = value as Record<string, unknown>;
  return typeof record.code === "string" && typeof record.message === "string" && typeof record.sessionLost === "boolean";
}
