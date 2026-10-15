import { describe, expect, it } from "vitest";
import fixture from "../__fixtures__/engine.json";
import type { AppState, EngineEvent } from "../engine";
import { initialStore, latestOnly, MAX_WARNINGS, needsResync, reduce, screenFor, type Store } from "../state";

const snapshot = fixture.appState as unknown as AppState;
const ev = (event: EngineEvent) => ({ kind: "event", event }) as const;
const ready = (s: Store) => {
  if (s.status !== "ready") throw new Error("expected ready");
  return s.view;
};

describe("reduce", () => {
  it("buffers events before the snapshot and applies them on top of it", () => {
    let s = reduce(initialStore, ev({ type: "elapsed", ms: 4242 }));
    expect(s.status).toBe("loading");
    s = reduce(s, { kind: "snapshot", state: snapshot });
    expect(ready(s).app.elapsedMs).toBe(4242);
  });

  it("a second snapshot replaces the view and clears warnings", () => {
    let s = reduce(initialStore, { kind: "snapshot", state: snapshot });
    s = reduce(s, ev({ type: "warning", code: "CAPTURE_BACKPRESSURE", message: "x" }));
    expect(ready(s).warnings).toHaveLength(1);
    s = reduce(s, { kind: "snapshot", state: { ...snapshot, elapsedMs: 5 } });
    expect(ready(s).warnings).toHaveLength(0);
    expect(ready(s).app.elapsedMs).toBe(5);
  });

  it("keeps only the newest warnings", () => {
    let s = reduce(initialStore, { kind: "snapshot", state: snapshot });
    for (let i = 0; i < 7; i++) {
      s = reduce(s, ev({ type: "warning", code: "CAPTURE_BACKPRESSURE", message: `w${i}` }));
    }
    const w = ready(s).warnings;
    expect(w).toHaveLength(MAX_WARNINGS);
    expect(w.map((x) => x.message)).toEqual(["w2", "w3", "w4", "w5", "w6"]);
  });

  it("publish opened, progress, done leaves publishing false", () => {
    let s = reduce(initialStore, { kind: "snapshot", state: snapshot });
    s = reduce(s, ev({ type: "publishOpened", url: "http://127.0.0.1:47821/", port: 47821 }));
    s = reduce(s, ev({ type: "publishProgress", bytesSent: 1, total: 2 }));
    s = reduce(s, ev({ type: "publishDone", webViewLink: "https://x", fileId: "f" }));
    expect(ready(s).app.publishing).toBe(false);
    expect(ready(s).publish.phase).toBe("done");
  });

  it("a snapshot with publishing true seeds the open phase", () => {
    const s = reduce(initialStore, { kind: "snapshot", state: { ...snapshot, publishing: true } });
    expect(ready(s).publish.phase).toBe("open");
  });
});

describe("needsResync", () => {
  it("re-reads the snapshot after a state change, which carries no recording", () => {
    expect(needsResync({ type: "stateChanged", from: "finalizing", to: "stopped" })).toBe(true);
  });

  it("does not re-read on the ~4 Hz clock", () => {
    expect(needsResync({ type: "elapsed", ms: 1 })).toBe(false);
  });
});

describe("screenFor", () => {
  it.each([
    ["idle", "recorder"],
    ["ready", "recorder"],
    ["recording", "recording"],
    ["finalizing", "recording"],
    ["stopped", "saved"],
    ["failed", "failed"],
  ] as const)("%s shows %s", (session, screen) => {
    expect(screenFor({ ...snapshot, session })).toBe(screen);
  });
});

describe("latestOnly", () => {
  it("drops a read that resolves after a newer one", async () => {
    const resolvers: ((value: string) => void)[] = [];
    const delivered: string[] = [];
    const sync = latestOnly(
      () => new Promise<string>((resolve) => resolvers.push(resolve)),
      (value) => delivered.push(value),
    );
    const first = sync();
    const second = sync();
    resolvers[1]?.("newer");
    resolvers[0]?.("older");
    await Promise.all([first, second]);
    expect(delivered).toEqual(["newer"]);
  });
});
