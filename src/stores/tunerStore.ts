// Tuner store — non-hot, non-frame-rate state.
//
// Hot-path PitchUpdate frames go through a window-scoped RingBuffer in
// `usePitchStream`, NOT through Zustand. This store mirrors only the slow-
// changing facts: capture lifecycle (`isCapturing`, `deviceName`), and the
// last note transition (`lastVoicedNoteLabel`) so screen readers can
// announce per-note rather than per-frame.
//
// Cross-references:
//   docs/design/DESIGN.md §7 (state model)

import { create } from "zustand";

export interface TunerState {
  deviceName: string | null;
  isCapturing: boolean;
  /** Stable label of the most recent voiced note transition, e.g. "A4". */
  lastVoicedNoteLabel: string | null;
  /** Last error surfaced by start_capture, if any. */
  startError: string | null;
}

export interface TunerActions {
  setCaptureStarted: (deviceName: string) => void;
  setCaptureStopped: () => void;
  setStartError: (msg: string | null) => void;
  setLastVoicedNoteLabel: (label: string | null) => void;
}

export type TunerStore = TunerState & TunerActions;

export const useTunerStore = create<TunerStore>((set) => ({
  deviceName: null,
  isCapturing: false,
  lastVoicedNoteLabel: null,
  startError: null,
  setCaptureStarted: (deviceName) => set({ deviceName, isCapturing: true, startError: null }),
  setCaptureStopped: () => set({ isCapturing: false }),
  setStartError: (msg) => set({ startError: msg }),
  setLastVoicedNoteLabel: (label) => set({ lastVoicedNoteLabel: label }),
}));
