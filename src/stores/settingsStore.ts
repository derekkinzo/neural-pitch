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
// Cross-references:
//   docs/design/DESIGN.md §7 (settings model)
//   docs/adr/0005-default-a4-440hz-with-user-override.md

import { create } from "zustand";
import {
  type AudioParams,
  type InstrumentHint,
  type TunerSettings,
  DEFAULT_SETTINGS,
  clampA4Hz,
  clampSmoothingMs,
} from "@/types/settings";

export interface SettingsActions {
  setA4Hz: (hz: number) => void;
  setInstrumentHint: (hint: InstrumentHint) => void;
  setSmoothingMs: (ms: number) => void;
  setAudioParams: (params: AudioParams | null) => void;
  reset: () => void;
}

export type SettingsState = TunerSettings & SettingsActions;

export const useSettingsStore = create<SettingsState>((set) => ({
  ...DEFAULT_SETTINGS,
  setA4Hz: (hz) => set({ a4Hz: clampA4Hz(hz) }),
  setInstrumentHint: (hint) => set({ instrumentHint: hint }),
  setSmoothingMs: (ms) => set({ smoothingMs: clampSmoothingMs(ms) }),
  setAudioParams: (params) => set({ audioParams: params }),
  reset: () => set({ ...DEFAULT_SETTINGS }),
}));

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
