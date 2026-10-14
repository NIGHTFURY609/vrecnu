import type { AppError, AppState, EngineEvent, ErrorCode } from "./engine";

export type PublishView =
  | { phase: "idle" }
  | { phase: "open" }
  | { phase: "uploading"; bytesSent: number; total: number }
  | { phase: "done"; webViewLink: string }
  | { phase: "failed"; error: AppError };

export interface EngineView {
  app: AppState;
  levels: { micDb: number; systemDb: number } | null;
  render: { id: number; fraction: number } | null;
  publish: PublishView;
  warnings: { code: ErrorCode; message: string }[];
}

/** Events that arrive before the first snapshot are buffered, then applied on top of it. */
export type Store =
  | { status: "loading"; buffered: EngineEvent[] }
  | { status: "ready"; view: EngineView };

export type Action = { kind: "snapshot"; state: AppState } | { kind: "event"; event: EngineEvent };

export const initialStore: Store = { status: "loading", buffered: [] };
export const MAX_WARNINGS = 5;

export function reduce(store: Store, action: Action): Store {
  if (action.kind === "snapshot") {
    const seeded = fromSnapshot(action.state);
    // A snapshot is authoritative: a late one (window re-created) replaces the view outright.
    const view = store.status === "loading" ? store.buffered.reduce(apply, seeded) : seeded;
    return { status: "ready", view };
  }
  if (store.status === "loading") {
    return { status: "loading", buffered: [...store.buffered, action.event] };
  }
  return { status: "ready", view: apply(store.view, action.event) };
}

function fromSnapshot(app: AppState): EngineView {
  return {
    app,
    levels: null,
    render: null,
    publish: app.publishing ? { phase: "open" } : { phase: "idle" },
    warnings: [],
  };
}

function apply(view: EngineView, event: EngineEvent): EngineView {
  switch (event.type) {
    case "stateChanged":
      return { ...view, app: { ...view.app, session: event.to } };
    case "elapsed":
      return { ...view, app: { ...view.app, elapsedMs: event.ms } };
    case "audioLevels":
      return { ...view, levels: { micDb: event.micDb, systemDb: event.systemDb } };
    case "stats":
      return {
        ...view,
        app: {
          ...view.app,
          stats: {
            framesIn: event.framesIn,
            framesWritten: event.framesWritten,
            framesDropped: event.framesDropped,
            framesDuplicated: event.framesDuplicated,
            encoderFps: event.encoderFps,
          },
        },
      };
    case "warning":
      return {
        ...view,
        warnings: [...view.warnings, { code: event.code, message: event.message }].slice(-MAX_WARNINGS),
      };
    case "renderProgress":
      return { ...view, render: { id: event.id, fraction: event.fraction } };
    case "renderDone":
      return { ...view, render: null, app: { ...view.app, recording: event.recording } };
    case "publishOpened":
      return { ...view, app: { ...view.app, publishing: true }, publish: { phase: "open" } };
    case "publishProgress":
      return { ...view, publish: { phase: "uploading", bytesSent: event.bytesSent, total: event.total } };
    case "publishDone":
      return { ...view, app: { ...view.app, publishing: false }, publish: { phase: "done", webViewLink: event.webViewLink } };
    case "publishFailed":
      return { ...view, app: { ...view.app, publishing: false }, publish: { phase: "failed", error: event.error } };
    case "failed":
      return { ...view, app: { ...view.app, error: event.error } };
  }
}
