import type { StorageProvider, UploadMetadata, UploadOptions } from "./storage-provider";

/**
 * In-memory StorageProvider fake. Lets packages/publish and packages/web wire up the full
 * record -> upload -> link UI before a real Google Cloud OAuth client exists — swap for
 * GoogleDriveProvider once a client id is configured.
 *
 * The returned link deliberately uses the reserved `.invalid` TLD, which is guaranteed never
 * to resolve. It previously returned a `drive.google.com` URL, which looked entirely real:
 * a user in demo mode was walked through "Upload → Copied → shareable with anyone" and handed
 * a plausible link to a file that does not exist. A fake link must *look* fake.
 */
export class MockStorageProvider implements StorageProvider {
  private connected = false;
  private readonly uploads = new Map<string, UploadMetadata>();
  private nextId = 0;

  /** Lets the UI state plainly that nothing was really uploaded. */
  readonly isMock = true;

  isConnected(): boolean {
    return this.connected;
  }

  async connect(): Promise<void> {
    this.connected = true;
  }

  async disconnect(): Promise<void> {
    this.connected = false;
  }

  async createFolder(name: string): Promise<string> {
    return `mock-folder-${name}`;
  }

  async uploadResumable(
    file: Blob,
    metadata: UploadMetadata,
    options: UploadOptions = {},
  ): Promise<{ fileId: string }> {
    const fileId = `mock-file-${++this.nextId}`;
    this.uploads.set(fileId, metadata);
    // Report progress in steps so the UI's progress bar is exercised, not skipped 0 -> 100.
    const steps = 4;
    for (let step = 1; step <= steps; step++) {
      options.onProgress?.({ bytesUploaded: Math.round((file.size * step) / steps), totalBytes: file.size });
    }
    return { fileId };
  }

  async createShareLink(fileId: string): Promise<string> {
    if (!this.uploads.has(fileId)) {
      throw new Error(`MockStorageProvider: unknown fileId "${fileId}"`);
    }
    return `https://example.invalid/vrecnu-demo/${fileId}`;
  }
}
