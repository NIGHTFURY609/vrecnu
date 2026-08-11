import type { ByteSource } from "./byte-source";

export interface UploadProgress {
  readonly bytesUploaded: number;
  readonly totalBytes: number;
}

export interface UploadMetadata {
  readonly name: string;
  readonly mimeType: string;
  /**
   * Destination folder. The caller supplies an id it already knows; providers must never
   * *search* for one, because listing the user's Drive is forbidden (AGENTS.md §5).
   */
  readonly folderId?: string;
}

/**
 * A resumable upload in flight. Handing this to the caller is what makes resume real: the
 * session URI outlives both the access token and the page, so an upload interrupted by a
 * network drop or a reload can continue instead of starting over.
 */
export interface ResumableSession {
  readonly uri: string;
  readonly fileName: string;
  readonly fileSize: number;
  readonly mimeType: string;
  readonly createdAt: number;
}

export interface UploadOptions {
  readonly onProgress?: (progress: UploadProgress) => void;
  readonly signal?: AbortSignal;
  /**
   * Resume this session instead of opening a new one. The provider probes the server for the
   * byte offset actually received rather than assuming where it left off.
   */
  readonly resumeSessionUri?: string;
  /** Called once the session exists, so the caller can persist it before any bytes move. */
  readonly onSessionStart?: (session: ResumableSession) => void;
  /** Called when the session is finished or permanently dead and should be forgotten. */
  readonly onSessionEnd?: (uri: string) => void;
}

/** Distinguishes "try again" from "this upload is over" for the UI. */
export class UploadError extends Error {
  constructor(
    message: string,
    readonly retryable: boolean,
    /** True when the session is gone and a retry must start from byte zero. */
    readonly sessionExpired = false,
  ) {
    super(message);
    this.name = "UploadError";
  }
}

/**
 * Drive is the first implementation, not a hardcoded dependency — UI/recording code must
 * only ever talk to storage through this interface (AGENTS.md §4).
 */
export interface StorageProvider {
  isConnected(): boolean;
  connect(): Promise<void>;
  disconnect(): Promise<void>;
  /**
   * Accepts a `Blob` (the PWA's OPFS file) or a {@link ByteSource} (the desktop's loopback
   * endpoint). One upload implementation serves both — see `byte-source.ts`.
   */
  uploadResumable(
    source: Blob | ByteSource,
    metadata: UploadMetadata,
    options?: UploadOptions,
  ): Promise<{ fileId: string }>;
  /** Sets "anyone with the link" sharing and returns the shareable webViewLink. */
  createShareLink(fileId: string): Promise<string>;
  /** Creates a folder and returns its id. Never searches — see {@link UploadMetadata.folderId}. */
  createFolder?(name: string): Promise<string>;
}
