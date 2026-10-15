import { Failed } from "./screens/Failed";
import { Recorder } from "./screens/Recorder";
import { Recording } from "./screens/Recording";
import { Saved } from "./screens/Saved";
import { screenFor, type EngineView } from "./state";
import { useEngine } from "./useEngine";

export function App() {
  const store = useEngine();
  return (
    <main className="flex min-h-screen flex-col gap-6 p-6 text-slate-100">
      <h1 className="text-xl font-semibold">vrecnu</h1>
      {store.status === "ready" ? <Screen view={store.view} /> : <p className="text-sm text-slate-400">Connecting…</p>}
    </main>
  );
}

function Screen({ view }: { view: EngineView }) {
  switch (screenFor(view.app)) {
    case "recorder":
      return <Recorder />;
    case "recording":
      return <Recording app={view.app} />;
    case "saved":
      return <Saved app={view.app} />;
    case "failed":
      return <Failed error={view.app.error} />;
  }
}
