// Analysis — wire-format types for the Phase 2.1 offline pYIN backend.
//
// The Rust shell emits camelCase per the existing recording.ts convention;
// the IPC boundary (`analyze_recording`, `get_contour`, `analysis-progress`)
// shares this shape with the Tier-5 Tauri-mock under tests/e2e/helpers.
//
// Cross-references:
//   docs/design/DESIGN.md §7.5 (Phase 2.1 frontend additions)
//   docs/design/DESIGN.md §8.3 (analysis_cache schema)
//   tests/e2e/helpers/tauri-mock.ts (MockAnalysisSummary / MockContourResult)

import type { RecordingId } from "@/types/recording";

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
