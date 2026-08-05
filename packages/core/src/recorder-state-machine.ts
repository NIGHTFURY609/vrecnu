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

const TRANSITIONS: Readonly<Record<RecorderState, Partial<Record<RecorderEventType, RecorderState>>>> = {
  idle: { REQUEST_PERMISSIONS: "requesting-permissions" },
  "requesting-permissions": { PERMISSIONS_GRANTED: "ready", PERMISSIONS_DENIED: "error" },
  ready: { START: "recording", ERROR: "error" },
  recording: { STOP: "stopping", ERROR: "error" },
  stopping: { CHUNKS_FLUSHED: "finalizing", ERROR: "error" },
  finalizing: { FINALIZED: "stopped", ERROR: "error" },
  stopped: { RESET: "idle" },
  error: { RESET: "idle" },
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
