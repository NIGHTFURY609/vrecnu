import { useEffect, useReducer } from "react";
import { initialStore, reduce, type Store } from "./state";
import { getSnapshot, onEngineEvent } from "./tauri";

/** Listens first, then reads the snapshot: an event landing in between is buffered, not lost. */
export function useEngine(): Store {
  const [store, dispatch] = useReducer(reduce, initialStore);
  useEffect(() => {
    let cancelled = false;
    let unlisten: (() => void) | undefined;
    onEngineEvent((event) => dispatch({ kind: "event", event }))
      .then(async (stop) => {
        if (cancelled) return stop();
        unlisten = stop;
        dispatch({ kind: "snapshot", state: await getSnapshot() });
      })
      .catch((err: unknown) => console.error("vrecnu: could not connect to the engine", err));
    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);
  return store;
}
