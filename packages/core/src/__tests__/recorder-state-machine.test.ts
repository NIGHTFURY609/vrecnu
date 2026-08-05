import { describe, expect, it } from "vitest";
import {
  IllegalRecorderTransitionError,
  INITIAL_RECORDER_STATE,
  recorderReducer,
} from "../recorder-state-machine";

describe("recorderReducer", () => {
  it("walks the full happy path idle -> stopped -> idle", () => {
    let state = INITIAL_RECORDER_STATE;
    state = recorderReducer(state, { type: "REQUEST_PERMISSIONS" });
    expect(state).toBe("requesting-permissions");
    state = recorderReducer(state, { type: "PERMISSIONS_GRANTED" });
    expect(state).toBe("ready");
    state = recorderReducer(state, { type: "START" });
    expect(state).toBe("recording");
    state = recorderReducer(state, { type: "STOP" });
    expect(state).toBe("stopping");
    state = recorderReducer(state, { type: "CHUNKS_FLUSHED" });
    expect(state).toBe("finalizing");
    state = recorderReducer(state, { type: "FINALIZED" });
    expect(state).toBe("stopped");
    state = recorderReducer(state, { type: "RESET" });
    expect(state).toBe("idle");
  });

  it("moves to error from any state that lists ERROR, and RESET returns to idle", () => {
    expect(recorderReducer("recording", { type: "ERROR", reason: "device lost" })).toBe("error");
    expect(recorderReducer("error", { type: "RESET" })).toBe("idle");
  });

  it("rejects a permissions denial by moving to error", () => {
    expect(
      recorderReducer("requesting-permissions", { type: "PERMISSIONS_DENIED", reason: "user declined" }),
    ).toBe("error");
  });

  it("throws IllegalRecorderTransitionError for an out-of-order event", () => {
    expect(() => recorderReducer("idle", { type: "START" })).toThrow(IllegalRecorderTransitionError);
    expect(() => recorderReducer("recording", { type: "REQUEST_PERMISSIONS" })).toThrow(
      IllegalRecorderTransitionError,
    );
  });

  it("does not allow STOP from ready (must be recording first)", () => {
    expect(() => recorderReducer("ready", { type: "STOP" })).toThrow(IllegalRecorderTransitionError);
  });
});
