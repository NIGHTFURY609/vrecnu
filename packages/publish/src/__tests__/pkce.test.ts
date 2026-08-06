import { describe, expect, it } from "vitest";
import { generateCodeChallenge, generateCodeVerifier } from "../pkce";

describe("pkce", () => {
  it("generates a verifier within RFC 7636 length + charset bounds", () => {
    const verifier = generateCodeVerifier();
    expect(verifier.length).toBeGreaterThanOrEqual(43);
    expect(verifier.length).toBeLessThanOrEqual(128);
    expect(verifier).toMatch(/^[A-Za-z0-9\-._~]+$/);
  });

  it("produces a different verifier each call", () => {
    expect(generateCodeVerifier()).not.toBe(generateCodeVerifier());
  });

  it("derives a deterministic, url-safe challenge from a verifier", async () => {
    const verifier = "test-verifier-value-1234567890-abcdefghijklmno";
    const challengeA = await generateCodeChallenge(verifier);
    const challengeB = await generateCodeChallenge(verifier);
    expect(challengeA).toBe(challengeB);
    expect(challengeA).toMatch(/^[A-Za-z0-9\-_]+$/);
    expect(challengeA).not.toContain("=");
  });

  it("derives different challenges from different verifiers", async () => {
    const challengeA = await generateCodeChallenge("verifier-one-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    const challengeB = await generateCodeChallenge("verifier-two-bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");
    expect(challengeA).not.toBe(challengeB);
  });
});
