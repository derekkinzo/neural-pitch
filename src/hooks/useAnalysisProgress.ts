// useAnalysisProgress — single-subscription hook for the
// `analysis-progress` Tauri event channel. Mirrors the Rust-side
// `AnalysisProgress` payload into the slow Zustand `analysisStore` at
// ~10 Hz. The mock-Tauri bridge surfaces the same channel via
// `pushAnalysisProgress(page, progress)` for E2E specs.
//
// Cross-references:
//   docs/design/DESIGN.md §7.5 (Phase 2.1 frontend additions)
//   src/hooks/useRecordingProgress.ts (precedent for the same pattern)

import { useEffect } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { useAnalysisStore } from "@/stores/analysisStore";
import type { AnalysisProgress } from "@/types/analysis";
import type { RecordingId } from "@/types/recording";

const CHANNEL = "analysis-progress";

interface WireProgress {
  recording_id?: string;
  recordingId?: string;
  percent?: number;
  status?: "running" | "finalizing" | "failed";
  error?: string;
}

function normalise(raw: WireProgress): AnalysisProgress {
  const recordingId: RecordingId = raw.recordingId ?? raw.recording_id ?? "";
  const percent = raw.percent ?? 0;
  const status = raw.status;
  const errorMsg = raw.error;
  const base: { recordingId: RecordingId; percent: number } = {
    recordingId,
    percent,
  };
  const withStatus = status !== undefined ? { ...base, status } : base;
  return errorMsg !== undefined ? { ...withStatus, error: errorMsg } : withStatus;
}

/**
 * Subscribe to the `analysis-progress` Tauri event for the lifetime of the
 * mounting component. Idempotent under React StrictMode double-invoke.
 */
export function useAnalysisProgress(): void {
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;

    type Listener = (payload: unknown) => void;
    type WindowWithHooks = Window & {
      __neuralPitchTestHooks?: {
        listeners: Map<string, Listener[]>;
      };
    };
    const w = window as WindowWithHooks;
    const hooks = w.__neuralPitchTestHooks;
    let unregisterTestListener: (() => void) | null = null;
    if (hooks !== undefined) {
      const fn: Listener = (payload) => {
        useAnalysisStore.getState().applyProgress(normalise(payload as WireProgress));
      };
      const list = hooks.listeners.get(CHANNEL) ?? [];
      list.push(fn);
      hooks.listeners.set(CHANNEL, list);
      unregisterTestListener = () => {
        const cur = hooks.listeners.get(CHANNEL) ?? [];
        hooks.listeners.set(
          CHANNEL,
          cur.filter((f) => f !== fn),
        );
      };
    } else {
      void listen<WireProgress>(CHANNEL, (event) => {
        useAnalysisStore.getState().applyProgress(normalise(event.payload));
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
             that simply resolves on IPC return — same effective UX. */
        });
    }

    return () => {
      cancelled = true;
      unregisterTestListener?.();
      if (unlisten !== null) unlisten();
    };
  }, []);
}
