/**
 * PKCE (RFC 7636) helpers for the GIS/PKCE public-client OAuth flow used by packages/publish.
 * Pure Web Crypto — no client secret, nothing platform-specific.
 */

function base64UrlEncode(bytes: Uint8Array): string {
  let binary = "";
  for (const byte of bytes) {
    binary += String.fromCharCode(byte);
  }
  return btoa(binary).replace(/\+/g, "-").replace(/\//g, "_").replace(/=+$/, "");
}

/** code_verifier: 43-128 chars from the unreserved URL-safe charset (RFC 7636 §4.1). */
export function generateCodeVerifier(byteLength = 64): string {
  const bytes = new Uint8Array(byteLength);
  crypto.getRandomValues(bytes);
  return base64UrlEncode(bytes).slice(0, 128);
}

/** code_challenge = BASE64URL(SHA256(code_verifier)), per RFC 7636 §4.2 (S256 method). */
export async function generateCodeChallenge(verifier: string): Promise<string> {
  const data = new TextEncoder().encode(verifier);
  const digest = await crypto.subtle.digest("SHA-256", data);
  return base64UrlEncode(new Uint8Array(digest));
}
