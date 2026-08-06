import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  clearUserClientId,
  DESKTOP_LOOPBACK_ORIGINS,
  looksLikeClientId,
  resolveCredentials,
  saveUserClientId,
} from "../credentials";

const BUILD_ID = "111-build.apps.googleusercontent.com";
const USER_ID = "222-user.apps.googleusercontent.com";

function stubLocalStorage(): Map<string, string> {
  const store = new Map<string, string>();
  vi.stubGlobal("window", {
    localStorage: {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => void store.set(k, v),
      removeItem: (k: string) => void store.delete(k),
    },
  });
  return store;
}

beforeEach(stubLocalStorage);
afterEach(() => vi.unstubAllGlobals());

describe("resolveCredentials", () => {
  it("reports none when nothing is configured", () => {
    expect(resolveCredentials(undefined)).toEqual({ clientId: null, source: "none" });
  });

  it("falls back to the build-time id", () => {
    expect(resolveCredentials(BUILD_ID)).toEqual({ clientId: BUILD_ID, source: "build" });
  });

  it("prefers the user's own id — a self-hoster must be able to override a hosted build", () => {
    saveUserClientId(USER_ID);
    expect(resolveCredentials(BUILD_ID)).toEqual({ clientId: USER_ID, source: "user" });
  });

  it("returns to the build default once the user's id is cleared", () => {
    saveUserClientId(USER_ID);
    clearUserClientId();
    expect(resolveCredentials(BUILD_ID)).toEqual({ clientId: BUILD_ID, source: "build" });
  });

  it("ignores a blank stored value", () => {
    saveUserClientId("   ");
    expect(resolveCredentials(BUILD_ID).source).toBe("build");
  });

  it("ignores a blank build-time value", () => {
    expect(resolveCredentials("  ").source).toBe("none");
  });

  it("survives localStorage being unavailable", () => {
    vi.stubGlobal("window", {
      localStorage: {
        getItem: () => {
          throw new Error("blocked");
        },
        setItem: () => {
          throw new Error("blocked");
        },
        removeItem: () => {},
      },
    });
    expect(() => saveUserClientId(USER_ID)).not.toThrow();
    expect(resolveCredentials(BUILD_ID).source).toBe("build");
  });
});

describe("looksLikeClientId", () => {
  it.each([
    ["1234567890-abc123.apps.googleusercontent.com", true],
    ["  1234567890-abc123.apps.googleusercontent.com  ", true],
    ["not-a-client-id", false],
    ["1234567890-abc123.apps.example.com", false],
    ["", false],
    // A client *secret* pasted by mistake must not be accepted as an id.
    ["GOCSPX-abcdefghijklmnop", false],
  ])("validates %s as %s", (input, expected) => {
    expect(looksLikeClientId(input)).toBe(expected);
  });
});

describe("DESKTOP_LOOPBACK_ORIGINS", () => {
  it("is exactly the five ports ADR-002 registers, in ascending order", () => {
    expect(DESKTOP_LOOPBACK_ORIGINS).toEqual([
      "http://127.0.0.1:47821",
      "http://127.0.0.1:47822",
      "http://127.0.0.1:47823",
      "http://127.0.0.1:47824",
      "http://127.0.0.1:47825",
    ]);
  });
});
