// TunerSettings — user-facing tuner configuration plus read-only audio
// parameters reported by the Rust shell.
//
// `a4Hz`, `instrumentHint`, and `smoothingMs` are user-writable. The audio
// params (`sampleRateHz`, `windowSamples`, `hopSamples`) are populated by a
// one-shot `invoke("get_audio_params")` after `start_capture` resolves and
// are surfaced read-only in the settings drawer.
//

export type InstrumentHint = "Generic" | "Voice" | "Guitar" | "Bass" | "Piano" | "Violin";

export const INSTRUMENT_HINTS: ReadonlyArray<InstrumentHint> = [
  "Generic",
  "Voice",
  "Guitar",
  "Bass",
  "Piano",
  "Violin",
];

/** Static Phase-1 prior ranges per InstrumentHint, sourced from
 *  Staged Phase-1 instrument ranges. Used by the StatusPill explicit-prior
 *  badge so the user can see the active search window without waiting for
 *  the first PriorNarrowed event from the engine. The "Generic" entry is the
 *  *fallback* shown before the auto-prior narrows; once a real range arrives
 *  via the `audio:backend` channel the StatusPill prefers `priorRange`.
 *
 *  Ranges are inclusive `[lowHz, highHz]`. */
export const INSTRUMENT_RANGE_TABLE: Readonly<Record<InstrumentHint, readonly [number, number]>> = {
  Generic: [80, 620],
  Voice: [70, 500],
  Guitar: [80, 1300],
  Bass: [40, 400],
  Piano: [27, 4200],
  Violin: [195, 3500],
};

/** Fallback range used by the auto-prior badge before `priorRange` is set. */
export const FALLBACK_GENERIC: readonly [number, number] = INSTRUMENT_RANGE_TABLE.Generic;

/** A4 reference presets in Hz. 440 is the default. */
export const A4_PRESETS: ReadonlyArray<number> = [415, 430, 435, 440, 442, 443, 466];

/** Range guard for the numeric A4 input. Out-of-range values are clamped at
 *  the store boundary rather than rejected loudly. */
export const A4_MIN_HZ = 410;
export const A4_MAX_HZ = 470;

/** Smoothing window range, in milliseconds. */
export const SMOOTHING_MIN_MS = 100;
export const SMOOTHING_MAX_MS = 500;
export const SMOOTHING_STEP_MS = 10;
export const SMOOTHING_DEFAULT_MS = 150;

export interface AudioParams {
  /** Sample rate of the active capture device, in Hz. */
  readonly sampleRateHz: number;
  /** Analysis window length, in samples. */
  readonly windowSamples: number;
  /** Hop length between successive analysis frames, in samples. */
  readonly hopSamples: number;
}

export interface TunerSettings {
  /** A4 reference frequency in Hz. Defaults to 440. */
  a4Hz: number;
  /** Instrument hint forwarded to the auto-prior selector. Defaults to
   *  "Generic" — a non-decision marker on day 1. */
  instrumentHint: InstrumentHint;
  /** Smoothing window in milliseconds (100–500). */
  smoothingMs: number;
  /** Read-only audio params populated after capture starts. */
  audioParams: AudioParams | null;
}

export const DEFAULT_SETTINGS: TunerSettings = {
  a4Hz: 440,
  instrumentHint: "Generic",
  smoothingMs: SMOOTHING_DEFAULT_MS,
  audioParams: null,
};

/** Clamp an A4 candidate value to the supported range. */
export function clampA4Hz(n: number): number {
  if (!Number.isFinite(n)) return 440;
  return Math.max(A4_MIN_HZ, Math.min(A4_MAX_HZ, n));
}

/** Clamp a smoothing-window candidate value to the supported range. */
export function clampSmoothingMs(n: number): number {
  if (!Number.isFinite(n)) return SMOOTHING_DEFAULT_MS;
  return Math.max(SMOOTHING_MIN_MS, Math.min(SMOOTHING_MAX_MS, Math.round(n)));
}
