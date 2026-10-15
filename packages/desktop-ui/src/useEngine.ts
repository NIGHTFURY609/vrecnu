import { useEffect, useReducer } from "react";
import { initialStore, latestOnly, needsResync, reduce, type Store } from "./state";
import { getSnapshot, onEngineEvent } from "./tauri";

/**
 * Listens first, then reads the snapshot: an event landing in between is buffered, not lost.
 * Re-reads the snapshot after every state change (see `needsResync`).
 */
export function useEngine(): Store {
  const [store, dispatch] = useReducer(reduce, initialStore);
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    const sync = latestOnly(getSnapshot, (state) => {
      // A cleanup landing while the snapshot was in flight must not apply it to a newer mount.
      if (!cancelled) dispatch({ kind: "snapshot", state });
    });
    const resync = () => {
      sync().catch((err: unknown) => console.error("vrecnu: could not read the engine state", err));
    };
    onEngineEvent((event) => {
      dispatch({ kind: "event", event });
      if (needsResync(event)) resync();
    })
      .then((stop) => {
        if (cancelled) return stop();
        unlisten = stop;
        resync();
      })
      .catch((err: unknown) => console.error("vrecnu: could not connect to the engine", err));
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);
  return store;
}
