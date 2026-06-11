// PitchUpdate — JSON shape emitted by the Rust DSP worker through the Tauri
// IPC channel.
//
// Mirrors `crates/neural-pitch-core/src/pipeline/sink.rs::PitchUpdate`. The
// fields are serde-serialised with default (snake_case) Rust field names, so
// the TypeScript surface keeps that exact casing rather than rewriting it on
// the hot path. The E2E mock (`tests/e2e/helpers/tauri-mock.ts`) emits the
// same shape via `pushPitchUpdate`.
//

export interface PitchUpdate {
  /** Absolute timestamp of the analysis frame's centre, in samples. */
  readonly timestamp_samples: number;
  /** Estimated fundamental frequency, in Hertz. */
  readonly f0_hz: number;
  /** Estimator confidence, normalised to `[0.0, 1.0]`. */
  readonly confidence: number;
  /** True if the estimator + voicing gate both reported voiced. */
  readonly voiced: boolean;
  /** Signed deviation in cents from the nearest equal-tempered note. */
  readonly smoothed_cents: number;
  /** MIDI number of the nearest equal-tempered note. */
  readonly target_midi: number;
  /** Equal-tempered Hertz value of `target_midi` at the configured a4. */
  readonly target_hz: number;
}

/** A neutral "no signal" snapshot. Used as the rAF default before any
 *  frame has been delivered, so paint() never sees `undefined`. */
export const SILENT_PITCH: PitchUpdate = {
  timestamp_samples: 0,
  f0_hz: 0,
  confidence: 0,
  voiced: false,
  smoothed_cents: 0,
  target_midi: 69,
  target_hz: 440,
};
