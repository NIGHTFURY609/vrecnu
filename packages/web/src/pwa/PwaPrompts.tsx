import { useEffect, useState, type JSX } from "react";
import { useRegisterSW } from "virtual:pwa-register/react";
import { Button } from "@vrecnu/ui";

/**
 * Chrome fires this instead of showing its own install UI, letting us choose the moment.
 * Not in lib.dom, so it is declared here.
 */
interface BeforeInstallPromptEvent extends Event {
  prompt(): Promise<void>;
  readonly userChoice: Promise<{ outcome: "accepted" | "dismissed" }>;
}

/**
 * The two prompts a PWA owes the user.
 *
 * The update prompt is deliberately *not* automatic: vrecnu can be mid-recording, and
 * swapping the service worker under a live capture risks the one thing the app must never
 * do. The user decides when to reload.
 *
 * Installing also matters beyond convenience — an installed origin is far more likely to be
 * granted persistent storage, which is what keeps a long recording from being evicted.
 */
export function PwaPrompts(): JSX.Element | null {
  const {
    needRefresh: [needRefresh, setNeedRefresh],
    updateServiceWorker,
  } = useRegisterSW();

  const [installEvent, setInstallEvent] = useState<BeforeInstallPromptEvent | null>(null);
  const [installDismissed, setInstallDismissed] = useState(false);

  useEffect(() => {
    const onBeforeInstall = (event: Event): void => {
      event.preventDefault();
      setInstallEvent(event as BeforeInstallPromptEvent);
    };
    const onInstalled = (): void => setInstallEvent(null);
    window.addEventListener("beforeinstallprompt", onBeforeInstall);
    window.addEventListener("appinstalled", onInstalled);
    return () => {
      window.removeEventListener("beforeinstallprompt", onBeforeInstall);
      window.removeEventListener("appinstalled", onInstalled);
    };
  }, []);

  const handleInstall = async (): Promise<void> => {
    if (!installEvent) {
      return;
    }
    await installEvent.prompt();
    await installEvent.userChoice;
    setInstallEvent(null);
  };

  const showInstall = installEvent !== null && !installDismissed;
  if (!needRefresh && !showInstall) {
    return null;
  }

  return (
    <div className="flex w-full flex-col gap-2" aria-live="polite">
      {needRefresh ? (
        <section className="flex flex-wrap items-center gap-2 rounded-md border border-blue-300 bg-blue-50 p-3 text-sm">
          <span className="text-blue-900">A new version of vrecnu is ready.</span>
          <Button size="sm" onClick={() => void updateServiceWorker(true)}>
            Reload
          </Button>
          <Button size="sm" variant="outline" onClick={() => setNeedRefresh(false)}>
            Later
          </Button>
          <span className="w-full text-xs text-blue-800">
            Finish any recording first — reloading ends the current session.
          </span>
        </section>
      ) : null}

      {showInstall ? (
        <section className="flex flex-wrap items-center gap-2 rounded-md border border-slate-300 bg-slate-50 p-3 text-sm">
          <span className="text-slate-800">Install vrecnu for offline use and more reliable storage.</span>
          <Button size="sm" variant="outline" onClick={() => void handleInstall()}>
            Install
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setInstallDismissed(true)}>
            Not now
          </Button>
        </section>
      ) : null}
    </div>
  );
}
