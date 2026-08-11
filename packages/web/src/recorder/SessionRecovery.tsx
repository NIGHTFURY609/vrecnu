import { useCallback, useEffect, useState, type JSX } from "react";
import { Button } from "@vrecnu/ui";
import {
  deleteRecording,
  deleteRecordings,
  getRecordingFile,
  listRecoverableRecordings,
  type RecordingSession,
} from "./opfs-sessions";
import {
  checkStorageHeadroom,
  formatBytes,
  formatDurationApprox,
  requestPersistentStorage,
  type PersistOutcome,
  type StorageHeadroom,
} from "./storage-headroom";

export interface SessionRecoveryProps {
  /** OPFS name of the recording this page owns, so it is never offered back as recoverable. */
  activeFileName?: string | null;
  onRecover: (file: File, session: RecordingSession) => void;
}

function describeSession(session: RecordingSession): string {
  const when = new Date(session.startedAt).toLocaleString();
  return `${when} · ${formatBytes(session.sizeBytes)}`;
}

/**
 * Two startup obligations the app previously skipped entirely, both from AGENTS.md §6 and
 * the ADR-003 amendment ("mandatory rather than optional"):
 *
 *  - Ask for persistent storage, so a long recording is not evicted mid-session.
 *  - Offer recordings left behind by a previous page load. A tab closed mid-recording leaves
 *    a perfectly good file on disk; not surfacing it loses work that was already saved.
 */
export function SessionRecovery({ activeFileName, onRecover }: SessionRecoveryProps): JSX.Element | null {
  const [orphans, setOrphans] = useState<RecordingSession[]>([]);
  const [headroom, setHeadroom] = useState<StorageHeadroom | null>(null);
  const [persistence, setPersistence] = useState<PersistOutcome | null>(null);
  const [busy, setBusy] = useState(false);
  const [dismissed, setDismissed] = useState(false);

  const refresh = useCallback(async (): Promise<void> => {
    const [found, space] = await Promise.all([
      listRecoverableRecordings(activeFileName ?? null),
      checkStorageHeadroom(),
    ]);
    setOrphans(found);
    setHeadroom(space);
  }, [activeFileName]);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      const outcome = await requestPersistentStorage();
      if (cancelled) {
        return;
      }
      setPersistence(outcome);
      await refresh();
    })();
    return () => {
      cancelled = true;
    };
  }, [refresh]);

  const handleRecover = async (session: RecordingSession): Promise<void> => {
    setBusy(true);
    try {
      const file = await getRecordingFile(session.fileName);
      if (file) {
        onRecover(file, session);
        setOrphans((prev) => prev.filter((item) => item.fileName !== session.fileName));
      }
    } finally {
      setBusy(false);
    }
  };

  const handleDiscard = async (session: RecordingSession): Promise<void> => {
    setBusy(true);
    try {
      await deleteRecording(session.fileName);
      setOrphans((prev) => prev.filter((item) => item.fileName !== session.fileName));
      await refresh();
    } finally {
      setBusy(false);
    }
  };

  const handleDiscardAll = async (): Promise<void> => {
    setBusy(true);
    try {
      await deleteRecordings(orphans.map((session) => session.fileName));
      setOrphans([]);
      await refresh();
    } finally {
      setBusy(false);
    }
  };

  const showHeadroom = headroom?.low === true;
  const showPersistWarning = persistence === "denied" && !dismissed;
  if (orphans.length === 0 && !showHeadroom && !showPersistWarning) {
    return null;
  }

  return (
    <div className="flex w-full flex-col gap-3" aria-live="polite">
      {orphans.length > 0 ? (
        <section className="rounded-md border border-amber-300 bg-amber-50 p-3 text-sm">
          <h2 className="font-medium text-amber-900">
            {orphans.length === 1
              ? "An earlier recording was left unfinished"
              : `${orphans.length} earlier recordings were left unfinished`}
          </h2>
          <p className="mt-1 text-amber-800">
            These were saved to disk but never finished. Recover one to play, download, or upload it.
          </p>
          <ul className="mt-2 flex flex-col gap-2">
            {orphans.map((session) => (
              <li key={session.fileName} className="flex flex-wrap items-center gap-2">
                <span className="font-mono text-xs text-amber-900">{describeSession(session)}</span>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={busy}
                  onClick={() => void handleRecover(session)}
                  aria-label={`Recover the recording from ${new Date(session.startedAt).toLocaleString()}`}
                >
                  Recover
                </Button>
                <Button
                  variant="outline"
                  size="sm"
                  disabled={busy}
                  onClick={() => void handleDiscard(session)}
                  aria-label={`Discard the recording from ${new Date(session.startedAt).toLocaleString()}`}
                >
                  Discard
                </Button>
              </li>
            ))}
          </ul>
          {orphans.length > 1 ? (
            <Button variant="outline" className="mt-2" disabled={busy} onClick={() => void handleDiscardAll()}>
              Discard all
            </Button>
          ) : null}
        </section>
      ) : null}

      {showHeadroom && headroom ? (
        <section className="rounded-md border border-red-300 bg-red-50 p-3 text-sm text-red-900">
          <h2 className="font-medium">Storage is running low</h2>
          <p className="mt-1">
            {headroom.availableBytes !== null ? formatBytes(headroom.availableBytes) : "An unknown amount"} of space
            is left
            {headroom.recordableSeconds !== null
              ? ` — roughly ${formatDurationApprox(headroom.recordableSeconds)} of recording.`
              : "."}{" "}
            Discard recordings you no longer need before starting a long session.
          </p>
        </section>
      ) : null}

      {showPersistWarning ? (
        <section className="rounded-md border border-gray-300 bg-gray-50 p-3 text-sm text-gray-700">
          <p>
            Your browser has not granted vrecnu persistent storage, so a recording could be evicted if the disk
            fills up. Installing vrecnu as an app usually grants it.
          </p>
          <Button variant="outline" className="mt-2" onClick={() => setDismissed(true)}>
            Dismiss
          </Button>
        </section>
      ) : null}
    </div>
  );
}
