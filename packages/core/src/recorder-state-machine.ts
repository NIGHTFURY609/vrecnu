export type RecorderState =
  | "idle"
  | "requesting-permissions"
  | "ready"
  | "recording"
  | "stopping"
  | "finalizing"
  | "stopped"
  | "error";

export type RecorderEventType =
  | "REQUEST_PERMISSIONS"
  | "PERMISSIONS_GRANTED"
  | "PERMISSIONS_DENIED"
  | "START"
  | "STOP"
  | "CHUNKS_FLUSHED"
  | "FINALIZED"
  | "RESET"
  | "ERROR";

export interface RecorderEvent {
  readonly type: RecorderEventType;
  readonly reason?: string;
}

export const INITIAL_RECORDER_STATE: RecorderState = "idle";

/**
 * `ERROR` is accepted from every non-terminal state on purpose. Anything that can fail must be
 * able to say so: a state that swallows `ERROR` strands the UI in a busy state with no way out,
 * because the caller's `catch` has nowhere to route the failure. `error` itself omits `ERROR`
 * (already there) and `RESET` is the only way forward from a terminal state.
 */
const TRANSITIONS: Readonly<Record<RecorderState, Partial<Record<RecorderEventType, RecorderState>>>> = {
  idle: { REQUEST_PERMISSIONS: "requesting-permissions", ERROR: "error" },
  "requesting-permissions": { PERMISSIONS_GRANTED: "ready", PERMISSIONS_DENIED: "error", ERROR: "error" },
  ready: { START: "recording", ERROR: "error" },
  recording: { STOP: "stopping", ERROR: "error" },
  stopping: { CHUNKS_FLUSHED: "finalizing", ERROR: "error" },
  finalizing: { FINALIZED: "stopped", ERROR: "error" },
  stopped: { RESET: "idle", ERROR: "error" },
  // Retrying straight from `error` is deliberate: the picker is on screen in that state, so
  // requiring a RESET round-trip first would leave "Start recording" silently inert.
  error: { RESET: "idle", REQUEST_PERMISSIONS: "requesting-permissions" },
};

export class IllegalRecorderTransitionError extends Error {
  constructor(
    public readonly state: RecorderState,
    public readonly event: RecorderEventType,
  ) {
    super(`Illegal recorder transition: ${event} from ${state}`);
    this.name = "IllegalRecorderTransitionError";
  }
}

/** Pure reducer — no side effects, no browser/Node APIs. Callers own the side effects each transition implies. */
export function recorderReducer(state: RecorderState, event: RecorderEvent): RecorderState {
  const next = TRANSITIONS[state][event.type];
  if (!next) {
    throw new IllegalRecorderTransitionError(state, event.type);
  }
  return next;
}
