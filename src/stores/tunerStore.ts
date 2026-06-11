// Tuner store — non-hot, non-frame-rate state.
//
// Hot-path PitchUpdate frames go through a window-scoped RingBuffer in
// `usePitchStream`, NOT through Zustand. This store mirrors only the slow-
// changing facts: capture lifecycle (`isCapturing`, `deviceName`), the
// last note transition (`lastVoicedNoteLabel`) so screen readers can
// announce per-note rather than per-frame, and device-status
// fields (auto-prior range, permission/disconnect status, negotiated
// audio format).
//

import { create } from "zustand";

export type DeviceStatus = "ok" | "permission_denied" | "disconnected" | "format_changed";

/** Top-level view selector — flips between the live tuner and the
 *  ear-training landing. The Practice button in the header sets this; the
 *  `/#training` URL hash exists only for Playwright deep-linking. */
export type TunerView = "tuner" | "training";

/** Default "requested" sample rate the shell asks the OS for. The negotiated
 *  rate may differ if the device cannot honour it; the engine resamples. */
export const REQUESTED_RATE_HZ_DEFAULT = 48_000;

export interface TunerState {
  deviceName: string | null;
  isCapturing: boolean;
  /** Stable label of the most recent voiced note transition, e.g. "A4". */
  lastVoicedNoteLabel: string | null;
  /** Last error surfaced by start_capture, if any. */
  startError: string | null;
  /** Active prior range in Hz (auto-prior badge). `null` while no
   *  PriorNarrowed event has been received. */
  priorRange: readonly [number, number] | null;
  /** Capture device status. Drives the permission-notice banner and the
   *  disconnect toast. */
  deviceStatus: DeviceStatus;
  /** Negotiated sample rate, in Hz, as reported by the audio backend. May
   *  differ from `requestedRateHz` when the OS picks a different format. */
  negotiatedRateHz: number | null;
  /** Negotiated channel count. `1` for mono input. */
  negotiatedChannels: number | null;
  /** Sample rate the shell asks for. Defaults to 48000. */
  requestedRateHz: number;
  /** Active top-level view. Defaults to "tuner". */
  view: TunerView;
}

export interface TunerActions {
  setCaptureStarted: (deviceName: string) => void;
  setCaptureStopped: () => void;
  setStartError: (msg: string | null) => void;
  setLastVoicedNoteLabel: (label: string | null) => void;
  setPriorRange: (range: readonly [number, number] | null) => void;
  setDeviceStatus: (status: DeviceStatus) => void;
  setNegotiatedFormat: (params: { rateHz: number; channels: number }) => void;
  clearDeviceError: () => void;
  setView: (view: TunerView) => void;
}

export type TunerStore = TunerState & TunerActions;

export const useTunerStore = create<TunerStore>((set) => ({
  deviceName: null,
  isCapturing: false,
  lastVoicedNoteLabel: null,
  startError: null,
  priorRange: null,
  deviceStatus: "ok",
  negotiatedRateHz: null,
  negotiatedChannels: null,
  requestedRateHz: REQUESTED_RATE_HZ_DEFAULT,
  view: "tuner",
  setCaptureStarted: (deviceName) =>
    set({ deviceName, isCapturing: true, startError: null, deviceStatus: "ok" }),
  setCaptureStopped: () => set({ isCapturing: false }),
  setStartError: (msg) => set({ startError: msg }),
  setLastVoicedNoteLabel: (label) => set({ lastVoicedNoteLabel: label }),
  setPriorRange: (range) => set({ priorRange: range }),
  setDeviceStatus: (status) => set({ deviceStatus: status }),
  setNegotiatedFormat: ({ rateHz, channels }) =>
    set({ negotiatedRateHz: rateHz, negotiatedChannels: channels }),
  clearDeviceError: () => set({ deviceStatus: "ok", startError: null }),
  setView: (view) => set({ view }),
}));
