import type { StorageProvider, UploadMetadata, UploadProgress } from "./storage-provider";

/**
 * In-memory StorageProvider fake. Lets packages/publish and packages/web wire up the full
 * record -> upload -> link UI before a real Google Cloud OAuth client exists (see the plan's
 * "manual follow-up" step) — swap for GoogleDriveProvider once VITE_GOOGLE_CLIENT_ID is set.
 */
export class MockStorageProvider implements StorageProvider {
  private connected = false;
  private readonly uploads = new Map<string, UploadMetadata>();
  private nextId = 0;

  isConnected(): boolean {
    return this.connected;
  }

  async connect(): Promise<void> {
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
    const fileId = `mock-file-${++this.nextId}`;
    this.uploads.set(fileId, metadata);
    onProgress?.({ bytesUploaded: file.size, totalBytes: file.size });
    return { fileId };
  }

  async createShareLink(fileId: string): Promise<string> {
    if (!this.uploads.has(fileId)) {
      throw new Error(`MockStorageProvider: unknown fileId "${fileId}"`);
    }
    return `https://drive.google.com/file/d/${fileId}/view?usp=sharing`;
  }
}
