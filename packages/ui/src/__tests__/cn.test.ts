import { describe, expect, it } from "vitest";
import { cn } from "../lib/cn";

describe("cn", () => {
  it("merges class lists and drops falsy values", () => {
    const disabled = false;
    expect(cn("a", disabled && "b", "c")).toBe("a c");
  });

  it("resolves conflicting Tailwind utilities, keeping the last one", () => {
    expect(cn("px-2", "px-4")).toBe("px-4");
  });
});
