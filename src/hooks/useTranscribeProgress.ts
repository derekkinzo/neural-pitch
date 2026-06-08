// useTranscribeProgress — single-subscription hook for the
// `transcribe-progress` Tauri event channel. Mirrors
// `useAnalysisProgress` and pushes the wire-format payload into the
// slow Zustand `transcriptionStore` at ~10 Hz. The mock-Tauri bridge
// surfaces the same channel via `pushTranscribeProgress(page, p)` for
// E2E specs.
//
//   src/hooks/useAnalysisProgress.ts (precedent for the same pattern)

import { useEffect } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getTestHooks, registerTestListener } from "@/lib/test-hooks";
import {
  __normaliseTranscribeProgress as normalise,
  useTranscriptionStore,
} from "@/stores/transcriptionStore";

const CHANNEL = "transcribe-progress";

interface WireProgress {
  recording_id?: string;
  recordingId?: string;
  percent?: number;
  status?: "running" | "finalizing" | "failed";
  error?: string;
}

/**
 * Subscribe to the `transcribe-progress` Tauri event for the lifetime of
 * the mounting component. Idempotent under React StrictMode double-invoke.
 */
export function useTranscribeProgress(): void {
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;

    const unregister = registerTestListener(CHANNEL, (payload) => {
      useTranscriptionStore.getState().applyProgress(normalise(payload as WireProgress));
    });
    if (getTestHooks() === undefined) {
      void listen<WireProgress>(CHANNEL, (event) => {
        useTranscriptionStore.getState().applyProgress(normalise(event.payload));
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
