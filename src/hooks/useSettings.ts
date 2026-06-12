// useSettings — selector + debounced `invoke("configure", ...)` bridge.
//
// React UI calls `update({...patch})` which (a) writes to `settingsStore`
// synchronously so the controls feel instant, (b) schedules a single
// `configure` IPC call after `DEBOUNCE_MS` of quiet. The Rust side documents
// `configure` as idempotent and atomic, so coalescing reconfigurations is
// safe.
//
// Pre-capture replay — settings adjusted while `isCapturing === false`
// (e.g. during the start_capture round-trip) are remembered as a
// `pendingFlush` flag and replayed on the false→true edge of
// `tunerStore.isCapturing` so an in-flight capture pickup does not silently
// drop the user's edits.
//

import { useCallback, useEffect, useRef } from "react";
import { invoke } from "@tauri-apps/api/core";
import { snapshotSettingsForIpc, useSettingsStore } from "@/stores/settingsStore";
import { useTunerStore } from "@/stores/tunerStore";
import { clampA4Hz, clampSmoothingMs, type InstrumentHint } from "@/types/settings";

const DEBOUNCE_MS = 150;

export interface UseSettingsApi {
  setA4Hz: (hz: number) => void;
  setInstrumentHint: (hint: InstrumentHint) => void;
  setSmoothingMs: (ms: number) => void;
}

/** Imperative settings API. Reads selectors from `useSettingsStore` directly
 *  so callers can subscribe at any granularity they like. */
export function useSettings(): UseSettingsApi {
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const pendingFlushRef = useRef<boolean>(false);

  const flush = useCallback(() => {
    if (!useTunerStore.getState().isCapturing) {
      // Capture has not yet attached. Mark that we owe a flush; the
      // tunerStore subscription below replays it on the false→true edge.
      pendingFlushRef.current = true;
      return;
    }
    pendingFlushRef.current = false;
    const payload = snapshotSettingsForIpc(useSettingsStore.getState());
    void invoke("configure", { settings: payload }).catch(() => {
      /* swallow: configure is best-effort; the next change reattempts. */
    });
  }, []);

  const schedule = useCallback(() => {
    if (timerRef.current !== null) clearTimeout(timerRef.current);
    timerRef.current = setTimeout(() => {
      timerRef.current = null;
      flush();
    }, DEBOUNCE_MS);
  }, [flush]);

  useEffect(() => {
    const t = timerRef;
    // Replay a pending flush when capture transitions from stopped to
    // live so edits made during the start_capture in-flight window are
    // not dropped.
    let prevCapturing = useTunerStore.getState().isCapturing;
    const unsub = useTunerStore.subscribe((state) => {
      const next = state.isCapturing;
      if (!prevCapturing && next && pendingFlushRef.current) {
        flush();
      }
      prevCapturing = next;
    });
    return () => {
      unsub();
      const handle = t.current;
      if (handle !== null) clearTimeout(handle);
    };
  }, [flush]);

  const setA4Hz = useCallback(
    (hz: number) => {
      useSettingsStore.getState().setA4Hz(clampA4Hz(hz));
      schedule();
    },
    [schedule],
  );

  const setInstrumentHint = useCallback(
    (hint: InstrumentHint) => {
      useSettingsStore.getState().setInstrumentHint(hint);
      schedule();
    },
    [schedule],
  );

  const setSmoothingMs = useCallback(
    (ms: number) => {
      useSettingsStore.getState().setSmoothingMs(clampSmoothingMs(ms));
      schedule();
    },
    [schedule],
  );

  return { setA4Hz, setInstrumentHint, setSmoothingMs };
}
