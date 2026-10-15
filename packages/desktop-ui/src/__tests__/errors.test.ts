import { describe, expect, it } from "vitest";
import type { AppError } from "../engine";
import { toAppError } from "../errors";

describe("toAppError", () => {
  it("passes an engine error through untouched", () => {
    const engine: AppError = {
      code: "ILLEGAL_STATE",
      message: "A recording is already running.",
      hint: null,
      detail: null,
      recoverable: true,
      sessionLost: false,
    };
    expect(toAppError(engine)).toBe(engine);
  });

  it("wraps a Tauri-level string rejection without claiming the recording was lost", () => {
    const err = toAppError("command start_recording not found");
    expect(err.code).toBe("INTERNAL");
    expect(err.detail).toBe("command start_recording not found");
    expect(err.sessionLost).toBe(false);
  });

  it("keeps an Error's message as the detail", () => {
    expect(toAppError(new Error("ipc closed")).detail).toBe("ipc closed");
  });
});
