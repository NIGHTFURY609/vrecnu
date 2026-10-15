import type { AppError } from "../engine";

/** One way to show an engine error: what happened, what to do, and the detail folded away. */
export function ErrorNote({ error }: { error: AppError }) {
  return (
    <div role="alert" className="rounded border border-red-900 bg-red-950/60 p-3 text-sm text-red-100">
      <p className="font-medium">{error.message}</p>
      {error.hint !== null && <p className="mt-1 text-red-200">{error.hint}</p>}
      {error.sessionLost && <p className="mt-1 text-red-200">The recording could not be saved.</p>}
      {error.detail !== null && (
        <details className="mt-2 text-xs text-red-300">
          <summary className="cursor-pointer">Details</summary>
          <pre className="mt-1 whitespace-pre-wrap break-all">{error.detail}</pre>
        </details>
      )}
    </div>
  );
}
