// useRecordingProgress — single-subscription hook for the
// `recording-progress` Tauri event channel. Mirrors the Rust-side
// `RecordingProgress` payload into the slow Zustand store at ~5 Hz.
//
// Test surface: when running under the E2E mock the helper exposes
// `window.__neuralPitchTestHooks.listeners` keyed by `"recording-progress"`,
// which a spec drives via `pushRecordingProgress(page, progress)`.
//
//   tests/e2e/helpers/tauri-mock.ts (synthetic event delivery)

import { useEffect } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getTestHooks, registerTestListener } from "@/lib/test-hooks";
import { useRecordingsStore } from "@/stores/recordingsStore";
import type { RecordingProgress } from "@/types/recording";

const CHANNEL = "recording-progress";

/** Snake_case wire-format mirroring the Rust serde output. The TS layer
 *  accepts either form. */
interface WireProgress {
  recording_id?: string;
  recordingId?: string;
  elapsed_ms?: number;
  elapsedMs?: number;
  sample_count?: number;
  sampleCount?: number;
  dropped_windows?: number;
  droppedWindows?: number;
  status?: "active" | "finalizing" | "failed";
  error?: string;
}

function normalise(raw: WireProgress): RecordingProgress {
  const recordingId = raw.recordingId ?? raw.recording_id ?? "";
  const elapsedMs = raw.elapsedMs ?? raw.elapsed_ms ?? 0;
  const sampleCount = raw.sampleCount ?? raw.sample_count ?? 0;
  const droppedWindows = raw.droppedWindows ?? raw.dropped_windows ?? 0;
  const status = raw.status ?? "active";
  const errorMsg = raw.error;
  const base = {
    recordingId,
    elapsedMs,
    sampleCount,
    droppedWindows,
    status,
  } as const;
  return errorMsg === undefined ? base : { ...base, error: errorMsg };
}

/**
 * Subscribe to the `recording-progress` Tauri event for the lifetime of
 * the mounting component. Idempotent under React StrictMode double-invoke.
 */
export function useRecordingProgress(): void {
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;

    const unregister = registerTestListener(CHANNEL, (payload) => {
      useRecordingsStore.getState().applyProgress(normalise(payload as WireProgress));
    });
    if (getTestHooks() === undefined) {
      void listen<WireProgress>(CHANNEL, (event) => {
        useRecordingsStore.getState().applyProgress(normalise(event.payload));
      })
        .then((u) => {
          if (cancelled) {
            u();
            return;
          }
          unlisten = u;
        })
        .catch(() => {
          /* swallow: a missing channel degrades to a static elapsed counter,
             which is the same surface as reduced-motion mode. */
        });
    }

    return () => {
      cancelled = true;
      unregister();
      if (unlisten !== null) unlisten();
    };
  }, []);
}
