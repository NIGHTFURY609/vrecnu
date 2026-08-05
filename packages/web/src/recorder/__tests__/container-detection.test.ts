import { afterEach, describe, expect, it, vi } from "vitest";
import { selectContainer } from "../container-detection";

describe("selectContainer", () => {
  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("picks the first CONTAINER_CANDIDATES entry MediaRecorder reports as supported", () => {
    vi.stubGlobal("MediaRecorder", {
      isTypeSupported: (mimeType: string) => mimeType === "video/webm;codecs=vp9,opus",
    });

    expect(selectContainer()).toEqual({ mimeType: "video/webm;codecs=vp9,opus", container: "webm" });
  });

  it("throws when no candidate is supported", () => {
    vi.stubGlobal("MediaRecorder", { isTypeSupported: () => false });

    expect(() => selectContainer()).toThrow(/No supported MediaRecorder mimeType/);
  });
});
