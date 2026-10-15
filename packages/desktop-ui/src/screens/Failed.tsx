import { useState } from "react";
import { Button } from "@vrecnu/ui";
import type { AppError } from "../engine";
import { toAppError } from "../errors";
import { resetSession } from "../tauri";
import { ErrorNote } from "./ErrorNote";
import { PRIMARY } from "./styles";

export function Failed({ error }: { error: AppError | null }) {
  const [resetError, setResetError] = useState<AppError | null>(null);
  return (
    <section className="flex flex-col gap-4">
      <h2 className="text-lg font-semibold">The recording stopped</h2>
      {error ? <ErrorNote error={error} /> : <p className="text-sm text-slate-300">vrecnu did not say why.</p>}
      {resetError && <ErrorNote error={resetError} />}
      <Button
        className={PRIMARY}
        onClick={() => {
          resetSession().catch((err: unknown) => setResetError(toAppError(err)));
        }}
      >
        Start over
      </Button>
    </section>
  );
}
