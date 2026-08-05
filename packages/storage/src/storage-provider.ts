export interface UploadProgress {
  readonly bytesUploaded: number;
  readonly totalBytes: number;
}

export interface UploadMetadata {
  readonly name: string;
  readonly mimeType: string;
}

/**
 * Drive is the first implementation, not a hardcoded dependency — UI/recording code must
 * only ever talk to storage through this interface (AGENTS.md §4).
 */
export interface StorageProvider {
  isConnected(): boolean;
  connect(): Promise<void>;
  disconnect(): Promise<void>;
  uploadResumable(
    file: Blob,
    metadata: UploadMetadata,
    onProgress?: (progress: UploadProgress) => void,
  ): Promise<{ fileId: string }>;
  /** Sets "anyone with the link" sharing and returns the shareable webViewLink. */
  createShareLink(fileId: string): Promise<string>;
}
