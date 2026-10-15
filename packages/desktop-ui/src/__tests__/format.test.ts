import { describe, expect, it } from "vitest";
import { fileName, formatBytes, formatElapsed } from "../format";

describe("formatElapsed", () => {
  it("reads as minutes, then hours, like the tray tooltip", () => {
    expect(formatElapsed(0)).toBe("0:00");
    expect(formatElapsed(59_999)).toBe("0:59");
    expect(formatElapsed(61_000)).toBe("1:01");
    expect(formatElapsed(3_600_000)).toBe("1:00:00");
    expect(formatElapsed(3_725_000)).toBe("1:02:05");
  });

  it("never shows a negative clock", () => {
    expect(formatElapsed(-5)).toBe("0:00");
  });
});

describe("formatBytes", () => {
  it("uses decimal units with one decimal from KB up", () => {
    expect(formatBytes(0)).toBe("0 B");
    expect(formatBytes(812)).toBe("812 B");
    expect(formatBytes(1_500)).toBe("1.5 KB");
    expect(formatBytes(4_100_000)).toBe("4.1 MB");
    expect(formatBytes(2_500_000_000)).toBe("2.5 GB");
  });
});

describe("fileName", () => {
  it("takes the last segment of a Windows or POSIX path", () => {
    expect(fileName("C:\\Users\\me\\AppData\\Local\\vrecnu\\recordings\\vrecnu-1.mp4")).toBe("vrecnu-1.mp4");
    expect(fileName("/tmp/vrecnu-2.mp4")).toBe("vrecnu-2.mp4");
    expect(fileName("vrecnu-3.mp4")).toBe("vrecnu-3.mp4");
  });
});
