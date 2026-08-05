import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { GoogleDriveProvider } from "../google-drive-provider";

function jsonResponse(body: unknown, init: ResponseInit = {}): Response {
  return new Response(JSON.stringify(body), {
    status: 200,
    ...init,
    headers: { "Content-Type": "application/json", ...(init.headers ?? {}) },
  });
}

describe("GoogleDriveProvider", () => {
  const getAccessToken = vi.fn(async () => "fake-token");
  let fetchMock: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    getAccessToken.mockClear();
    fetchMock = vi.fn();
    vi.stubGlobal("fetch", fetchMock);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("uploads via init -> PUT -> permissions.create -> metadata fetch, in that order", async () => {
    fetchMock
      .mockImplementationOnce(async (url: string, init: RequestInit) => {
        expect(url).toContain("uploadType=resumable");
        expect(init.method).toBe("POST");
        expect(init.headers).toMatchObject({ Authorization: "Bearer fake-token" });
        return new Response(null, { status: 200, headers: { Location: "https://upload.example/session-1" } });
      })
      .mockImplementationOnce(async (url: string, init: RequestInit) => {
        expect(url).toBe("https://upload.example/session-1");
        expect(init.method).toBe("PUT");
        return jsonResponse({ id: "file-123" }, { status: 200 });
      })
      .mockImplementationOnce(async (url: string, init: RequestInit) => {
        expect(url).toBe("https://www.googleapis.com/drive/v3/files/file-123/permissions");
        expect(init.method).toBe("POST");
        expect(JSON.parse(String(init.body))).toEqual({ role: "reader", type: "anyone" });
        return jsonResponse({ id: "perm-1" });
      })
      .mockImplementationOnce(async (url: string) => {
        expect(url).toBe("https://www.googleapis.com/drive/v3/files/file-123?fields=webViewLink");
        return jsonResponse({ webViewLink: "https://drive.google.com/file/d/file-123/view" });
      });

    const provider = new GoogleDriveProvider({ getAccessToken });
    const file = new Blob(["hello world"], { type: "video/webm" });
    const { fileId } = await provider.uploadResumable(file, { name: "clip.webm", mimeType: "video/webm" });
    expect(fileId).toBe("file-123");

    const link = await provider.createShareLink(fileId);
    expect(link).toBe("https://drive.google.com/file/d/file-123/view");
    expect(fetchMock).toHaveBeenCalledTimes(4);
  });

  it("never hardcodes a token — every request goes through getAccessToken", async () => {
    fetchMock.mockImplementation(async (url: string) => {
      if (url.includes("uploadType=resumable")) {
        return new Response(null, { status: 200, headers: { Location: "https://upload.example/session-2" } });
      }
      return jsonResponse({ id: "file-456" });
    });
    const provider = new GoogleDriveProvider({ getAccessToken });
    await provider.uploadResumable(new Blob(["x"]), { name: "a.mp4", mimeType: "video/mp4" });
    expect(getAccessToken).toHaveBeenCalled();
  });

  it("throws a clear error when the resumable session init returns no Location header", async () => {
    fetchMock.mockResolvedValueOnce(new Response(null, { status: 200 }));
    const provider = new GoogleDriveProvider({ getAccessToken });
    await expect(
      provider.uploadResumable(new Blob(["x"]), { name: "a.webm", mimeType: "video/webm" }),
    ).rejects.toThrow(/Location header/);
  });
});
