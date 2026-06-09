// useStemProgressSubscription — single-subscription hook for the
// `separate-progress` Tauri event channel. Mirrors
// `useTranscribeProgress` and pushes the wire-format payload through
// the slow Zustand `stemsStore` (stage transitions only) AND the
// process-wide ref bus (per-frame `percent`). The mock-Tauri bridge
// surfaces the same channel via `pushStemsProgress(page, p)` for E2E.
//
//   src/hooks/useTranscribeProgress.ts (precedent for the same pattern)

import { useEffect } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getTestHooks, registerTestListener } from "@/lib/test-hooks";
import { __normaliseStemsProgress as normalise, useStemsStore } from "@/stores/stemsStore";

const CHANNEL = "separate-progress";

interface WireProgress {
  recording_id?: string;
  recordingId?: string;
  stage?: "vocals" | "drums" | "bass" | "other" | "finalizing";
  percent?: number;
}

/**
 * Subscribe to the `separate-progress` Tauri event for the lifetime of
 * the mounting component. Idempotent under React StrictMode double-invoke.
 */
export function useStemProgressSubscription(): void {
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;

    const unregister = registerTestListener(CHANNEL, (payload) => {
      useStemsStore.getState().applyProgress(normalise(payload as WireProgress));
    });
    if (getTestHooks() === undefined) {
      void listen<WireProgress>(CHANNEL, (event) => {
        useStemsStore.getState().applyProgress(normalise(event.payload));
      })
        .then((u) => {
          if (cancelled) {
            u();
            return;
          }
          unlisten = u;
        })
        .catch(() => {
          /* swallow: a missing channel degrades to a static progress bar
             that simply resolves on IPC return. */
        });
    }

    return () => {
      cancelled = true;
      unregister();
      if (unlisten !== null) unlisten();
    };
  }, []);
}
