// Analysis — wire-format types for the Phase 2.1 offline pYIN backend.
//
// The Rust shell emits camelCase per the existing recording.ts convention;
// the IPC boundary (`analyze_recording`, `get_contour`, `analysis-progress`)
// shares this shape with the Tier-5 Tauri-mock under tests/e2e/helpers.
//
//   tests/e2e/helpers/tauri-mock.ts (MockAnalysisSummary / MockContourResult)

import type { RecordingId } from "@/types/recording";

/** Phase 2.3 — vocal-range report carried alongside the summary card.
 *
 *  The analyzer derives a 5th/95th-percentile "comfortable" range plus the
 *  raw min/max voiced MIDI. `voicedFrameCount` gates the readout's empty
 *  branch (we need at least ~5 s of voiced audio at a 50 ms hop ≈ 250
 *  frames before the percentile estimate is meaningful). `voiceTypeHints`
 *  follows the New Grove vocal-range conventions — informational only,
 *  NOT a vocal-coach assessment.
 *
 */
export interface RangeReport {
  /** 5th percentile of voiced MIDI. */
  readonly comfortableLowMidi: number;
  /** 95th percentile of voiced MIDI. */
  readonly comfortableHighMidi: number;
  /** Min voiced MIDI across the full recording. */
  readonly fullLowMidi: number;
  /** Max voiced MIDI across the full recording. */
  readonly fullHighMidi: number;
  /** Total voiced frame count — gates the "Not enough voiced material"
   *  empty branch in `RangeReadout`. */
  readonly voicedFrameCount: number;
  /** Voice-type hint labels (e.g. ["Alto", "Mezzo-soprano"]) per the New
   *  Grove Dictionary of Music vocal-range conventions. */
  readonly voiceTypeHints: readonly string[];
}

/** Phase 2.3 — single vibrato analysis window. */
export interface VibratoWindow {
  /** Window center timestamp in milliseconds. */
  readonly tMs: number;
  /** Estimated vibrato rate in Hz for this window. */
  readonly rateHz: number;
  /** Estimated vibrato extent in cents (peak-to-peak / 2). */
  readonly extentCents: number;
  /** 0..1 — drives per-window dot color in the readout strip. */
  readonly confidence: number;
}

/** Phase 2.3 — vibrato report carried alongside the summary card. */
export interface VibratoReport {
  /** Median rate (Hz) across all windows. */
  readonly medianRateHz: number;
  /** Median extent (cents) across all windows. */
  readonly medianExtentCents: number;
  /** Fraction of voiced material classified as vibrato (0..1). */
  readonly vibratoRatio: number;
  /** Per-window samples — drives the dot strip below the rate bar. */
  readonly windows: readonly VibratoWindow[];
}

/** Per-recording summary card payload. The Rust analyzer derives the median
 *  pitch (MIDI), median cents off (relative to the nearest equal-tempered
 *  note), and the voiced-frame ratio from the full pYIN contour. */
export interface AnalysisSummary {
  readonly recordingId: RecordingId;
  /** MIDI number of the median voiced pitch (e.g. 69 == A4). */
  readonly medianMidi: number;
  /** Signed cents from the equal-tempered `medianMidi` reference. Display is
   *  rounded to 1 decimal at render time. */
  readonly medianCents: number;
  /** Voiced ratio in `[0, 1]`. Display surfaces it as a percentage. */
  readonly voicedRatio: number;
  /** True iff the analyzer hit the cache; the AnalysisSummary card surfaces
   *  this as a "Cached" / "Fresh" badge. The pYIN backend resolves this from
   *  the `analysis_cache` SHA-256 row keyed on `(audio_blob_sha, version)`. */
  readonly wasCached: boolean;
  /** Analyzer version stamp — part of the cache key alongside the recording
   *  id. Specs assert via the `${recordingId}:${analyzerVersion}` composite. */
  readonly analyzerVersion: string;
  /** Phase 2.3 — vocal range report. Optional because older cached rows
   *  pre-date the field; readouts treat `undefined` as the empty branch. */
  readonly range?: RangeReport;
  /** Phase 2.3 — vibrato report. Optional for the same reason. */
  readonly vibrato?: VibratoReport;
}

/** A single voiced/unvoiced frame in the contour timeline. */
export interface ContourFrame {
  /** Frame center timestamp in milliseconds, relative to the start of the
   *  recording. */
  readonly tMs: number;
  /** Signed deviation in cents from the recording's median pitch. */
  readonly centsFromMedian: number;
  /** True iff the analyzer + voicing gate both reported voiced. */
  readonly voiced: boolean;
}

/** Full contour payload — fetched lazily after the summary lands. */
export interface ContourResult {
  readonly recordingId: RecordingId;
  readonly analyzerVersion: string;
  readonly medianMidi: number;
  readonly medianCents: number;
  readonly voicedRatio: number;
  readonly frames: readonly ContourFrame[];
}

/** Streaming progress payload — emitted at ~10 Hz over the
 *  `analysis-progress` Tauri channel while pYIN/PESTO is running. */
export interface AnalysisProgress {
  readonly recordingId: RecordingId;
  /** Percent complete in `[0, 100]`. The card flips back to the numeric
   *  readouts when the in-flight `analyze_recording` promise resolves. */
  readonly percent: number;
  readonly status?: "running" | "finalizing" | "failed";
  readonly error?: string;
}
