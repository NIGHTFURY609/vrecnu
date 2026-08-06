/**
 * OPFS session bookkeeping: naming, garbage collection, and crash recovery
 * (ADR-003 §3.1 — "On startup, if chunks from an unfinished session are present, offer
 * crash recovery").
 *
 * Without this, every recording permanently consumes origin quota and the user has no way
 * to see it or reclaim it — the failure the ADR-003 amendment's "Consequences → Harder"
 * section warns about. A tab closed mid-recording leaves a perfectly good file behind;
 * refusing to surface it loses work that was already safely on disk.
 */

import type { ContainerFormat } from "@vrecnu/core";

const FILE_PREFIX = "vrecnu-";
const RECORDING_PATTERN = /^vrecnu-(\d+)\.(webm|mp4)$/;

/**
 * TypeScript's DOM lib does not expose the async-iterable half of FileSystemDirectoryHandle
 * in every configuration, so the iteration surface is declared locally rather than relying
 * on `lib.dom.asynciterable` being present.
 */
interface IterableDirectoryHandle extends FileSystemDirectoryHandle {
  values(): AsyncIterableIterator<FileSystemHandle>;
}

export interface RecordingSession {
  readonly fileName: string;
  readonly container: ContainerFormat;
  readonly startedAt: number;
  readonly sizeBytes: number;
  readonly lastModified: number;
}

export function buildRecordingFileName(container: ContainerFormat, startedAt: number = Date.now()): string {
  return `${FILE_PREFIX}${startedAt}.${container}`;
}

async function getRoot(): Promise<IterableDirectoryHandle | null> {
  if (!navigator.storage?.getDirectory) {
    return null;
  }
  try {
    return (await navigator.storage.getDirectory()) as IterableDirectoryHandle;
  } catch {
    return null;
  }
}

/**
 * Every vrecnu recording currently on disk, newest first. The caller decides which of these
 * are "orphans" — this function deliberately does not guess, because the in-progress
 * recording is indistinguishable from an abandoned one by file metadata alone.
 */
export async function listRecordings(): Promise<RecordingSession[]> {
  const root = await getRoot();
  if (!root || typeof root.values !== "function") {
    return [];
  }

  const sessions: RecordingSession[] = [];
  try {
    for await (const handle of root.values()) {
      if (handle.kind !== "file") {
        continue;
      }
      const match = RECORDING_PATTERN.exec(handle.name);
      if (!match) {
        continue;
      }
      try {
        const file = await (handle as FileSystemFileHandle).getFile();
        sessions.push({
          fileName: handle.name,
          container: match[2] as ContainerFormat,
          startedAt: Number(match[1]),
          sizeBytes: file.size,
          lastModified: file.lastModified,
        });
      } catch {
        // A file still held open by a live sync access handle can refuse getFile(); skip it
        // rather than failing the whole scan.
      }
    }
  } catch {
    return sessions;
  }

  return sessions.sort((a, b) => b.startedAt - a.startedAt);
}

/**
 * Recordings left behind by a previous page load. `exclude` is the session this page owns,
 * which must never be offered back to the user as recoverable.
 *
 * Zero-byte files are dropped: they are sessions that died before the first chunk landed and
 * contain nothing to recover.
 */
export async function listRecoverableRecordings(exclude?: string | null): Promise<RecordingSession[]> {
  const all = await listRecordings();
  return all.filter((session) => session.fileName !== exclude && session.sizeBytes > 0);
}

export async function deleteRecording(fileName: string): Promise<void> {
  const root = await getRoot();
  if (!root) {
    return;
  }
  try {
    await root.removeEntry(fileName);
  } catch {
    // Already gone, or still locked by a worker that has not terminated yet. Either way the
    // caller has nothing useful to do about it.
  }
}

export async function deleteRecordings(fileNames: readonly string[]): Promise<void> {
  await Promise.all(fileNames.map((name) => deleteRecording(name)));
}

export async function getRecordingFile(fileName: string): Promise<File | null> {
  const root = await getRoot();
  if (!root) {
    return null;
  }
  try {
    const handle = await root.getFileHandle(fileName);
    return await handle.getFile();
  } catch {
    return null;
  }
}

export async function totalRecordingBytes(): Promise<number> {
  const sessions = await listRecordings();
  return sessions.reduce((sum, session) => sum + session.sizeBytes, 0);
}
