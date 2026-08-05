import type { StorageProvider, UploadMetadata, UploadProgress } from "./storage-provider";

const DRIVE_UPLOAD_ENDPOINT = "https://www.googleapis.com/upload/drive/v3/files";
const DRIVE_API_ENDPOINT = "https://www.googleapis.com/drive/v3/files";
/** Must be a multiple of 256 KiB per the Drive resumable-upload spec. */
const UPLOAD_CHUNK_SIZE = 8 * 1024 * 1024;

export interface GoogleDriveProviderOptions {
  /**
   * GIS access tokens last ~1 hour with no refresh token (AGENTS.md §6). A large upload on a
   * slow connection can outlive one, so every request re-acquires a token through this
   * callback rather than being constructed with a fixed token string.
   */
  getAccessToken: () => Promise<string>;
}

/**
 * Talks to Drive REST v3 using only drive.file-scope-compatible endpoints: create (this
 * app's own upload), permissions.create, and a metadata read of a file this app created.
 * Never lists or searches existing files (AGENTS.md §5).
 */
export class GoogleDriveProvider implements StorageProvider {
  private connected = false;

  constructor(private readonly options: GoogleDriveProviderOptions) {}

  isConnected(): boolean {
    return this.connected;
  }

  async connect(): Promise<void> {
    await this.options.getAccessToken();
    this.connected = true;
  }

  async disconnect(): Promise<void> {
    this.connected = false;
  }

  async uploadResumable(
    file: Blob,
    metadata: UploadMetadata,
    onProgress?: (progress: UploadProgress) => void,
  ): Promise<{ fileId: string }> {
    const sessionUri = await this.initResumableSession(file, metadata);
    const fileId = await this.putChunks(sessionUri, file, onProgress);
    return { fileId };
  }

  async createShareLink(fileId: string): Promise<string> {
    const token = await this.options.getAccessToken();
    const permResponse = await fetch(`${DRIVE_API_ENDPOINT}/${fileId}/permissions`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${token}`,
        "Content-Type": "application/json",
      },
      body: JSON.stringify({ role: "reader", type: "anyone" }),
    });
    if (!permResponse.ok) {
      throw new Error(`Drive permissions.create failed: ${permResponse.status} ${permResponse.statusText}`);
    }

    const metaResponse = await fetch(`${DRIVE_API_ENDPOINT}/${fileId}?fields=webViewLink`, {
      headers: { Authorization: `Bearer ${token}` },
    });
    if (!metaResponse.ok) {
      throw new Error(`Drive file metadata fetch failed: ${metaResponse.status} ${metaResponse.statusText}`);
    }
    const { webViewLink } = (await metaResponse.json()) as { webViewLink: string };
    return webViewLink;
  }

  private async initResumableSession(file: Blob, metadata: UploadMetadata): Promise<string> {
    const token = await this.options.getAccessToken();
    const response = await fetch(`${DRIVE_UPLOAD_ENDPOINT}?uploadType=resumable`, {
      method: "POST",
      headers: {
        Authorization: `Bearer ${token}`,
        "Content-Type": "application/json; charset=UTF-8",
        "X-Upload-Content-Type": metadata.mimeType,
        "X-Upload-Content-Length": String(file.size),
      },
      body: JSON.stringify({ name: metadata.name }),
    });
    if (!response.ok) {
      throw new Error(`Drive resumable session init failed: ${response.status} ${response.statusText}`);
    }
    const location = response.headers.get("Location");
    if (!location) {
      throw new Error("Drive resumable session init did not return a Location header");
    }
    return location;
  }

  private async putChunks(
    sessionUri: string,
    file: Blob,
    onProgress?: (progress: UploadProgress) => void,
  ): Promise<string> {
    const totalBytes = file.size;
    let offset = 0;

    while (offset < totalBytes) {
      const end = Math.min(offset + UPLOAD_CHUNK_SIZE, totalBytes);
      const chunk = file.slice(offset, end);
      // Re-acquire per chunk: the resumable session URI (not the token) is what survives re-auth.
      const token = await this.options.getAccessToken();
      const response = await fetch(sessionUri, {
        method: "PUT",
        headers: {
          Authorization: `Bearer ${token}`,
          "Content-Range": `bytes ${offset}-${end - 1}/${totalBytes}`,
        },
        body: chunk,
      });

      if (response.status === 308) {
        offset = end;
        onProgress?.({ bytesUploaded: offset, totalBytes });
        continue;
      }
      if (response.status === 200 || response.status === 201) {
        const { id } = (await response.json()) as { id: string };
        onProgress?.({ bytesUploaded: totalBytes, totalBytes });
        return id;
      }
      throw new Error(`Drive chunk upload failed: ${response.status} ${response.statusText}`);
    }

    throw new Error("Drive resumable upload ended without a completion response");
  }
}
