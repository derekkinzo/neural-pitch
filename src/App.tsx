// App root — mounts the live tuner OR the ear-training screen
// based on `tunerStore.view`. The header's Practice button flips the
// view; a `/#training` URL hash also routes into the training screen so
// Playwright (and a power-user with a bookmark) can deep-link past the
// tuner. The hash itself is consumed once on mount and then cleared so
// later view changes do not loop back through the URL bar.

import { useEffect, type ReactNode } from "react";
import { Tuner } from "@/components/Tuner";
import { Training } from "@/components/training/Training";
import { useTunerStore } from "@/stores/tunerStore";

export function App(): ReactNode {
  const view = useTunerStore((s) => s.view);
  const setView = useTunerStore((s) => s.setView);

  useEffect(() => {
    if (typeof window === "undefined") return;
    if (window.location.hash === "#training") {
      setView("training");
      // Clear the hash so a subsequent `setView("tuner")` is not
      // shadowed by the deep-link on the next mount/reload.
      try {
        history.replaceState(null, "", window.location.pathname + window.location.search);
      } catch {
        /* swallow: history mutation is best-effort */
      }
    }
  }, [setView]);

  return view === "training" ? <Training /> : <Tuner />;
}
