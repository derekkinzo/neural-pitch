// Settings store — low-frequency, user-writable configuration.
//
// Two-layer write strategy: the React UI calls `setSettings(patch)` which
// updates the local store synchronously. A separate hook
// (`hooks/useSettings.ts`) debounces those mutations and forwards them to
// the Rust shell via `invoke("configure", { settings })`.
//
// The audio params (`sampleRateHz`, `windowSamples`, `hopSamples`) are
// reported once after `start_capture` resolves and live read-only on the same
// store for cheap UI consumption.
//
// Persistence: `noteLabelMode` is persisted to localStorage (the
// ear-training drill UI's only persistent setting on the page side). `a4Hz`,
// `instrumentHint`, and `smoothingMs` are persisted via the Rust shell
// through the debounced `configure` IPC, so they intentionally live
// outside the partialize whitelist below.
//

import { create } from "zustand";
import { persist, createJSONStorage } from "zustand/middleware";
import {
  type AudioParams,
  type InstrumentHint,
  type NoteLabelMode,
  type TunerSettings,
  DEFAULT_SETTINGS,
  clampA4Hz,
  clampSmoothingMs,
} from "@/types/settings";

export interface SettingsActions {
  setA4Hz: (hz: number) => void;
  setInstrumentHint: (hint: InstrumentHint) => void;
  setSmoothingMs: (ms: number) => void;
  setNoteLabelMode: (mode: NoteLabelMode) => void;
  setAudioParams: (params: AudioParams | null) => void;
  reset: () => void;
}

export type SettingsState = TunerSettings & SettingsActions;

const NOTE_LABEL_MODE_STORAGE_KEY = "neural-pitch.settings.noteLabelMode.v1";

export const useSettingsStore = create<SettingsState>()(
  persist(
    (set) => ({
      ...DEFAULT_SETTINGS,
      setA4Hz: (hz) => set({ a4Hz: clampA4Hz(hz) }),
      setInstrumentHint: (hint) => set({ instrumentHint: hint }),
      setSmoothingMs: (ms) => set({ smoothingMs: clampSmoothingMs(ms) }),
      setNoteLabelMode: (mode) => set({ noteLabelMode: mode }),
      setAudioParams: (params) => set({ audioParams: params }),
      reset: () => set({ ...DEFAULT_SETTINGS }),
    }),
    {
      name: NOTE_LABEL_MODE_STORAGE_KEY,
      storage: createJSONStorage(() => localStorage),
      // Whitelist: only `noteLabelMode` is persisted on the page side.
      // The Rust shell owns the rest of the settings (A4 / instrument
      // hint / smoothing) via the `configure` IPC.
      partialize: (state) => ({ noteLabelMode: state.noteLabelMode }),
    },
  ),
);

/** Read-only snapshot for IPC payloads. The Rust side names fields in
 *  snake_case; we hand-shape that here to avoid a dependency on ts-rs. */
export interface SettingsIpcPayload {
  readonly a4_hz: number;
  readonly instrument_hint: InstrumentHint;
  readonly smoothing_ms: number;
}

export function snapshotSettingsForIpc(s: TunerSettings): SettingsIpcPayload {
  return {
    a4_hz: s.a4Hz,
    instrument_hint: s.instrumentHint,
    smoothing_ms: s.smoothingMs,
  };
}
