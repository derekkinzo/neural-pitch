// Transcription — wire-format types for the Phase 3 Basic Pitch backend.
//
// Mirrors `src/types/analysis.ts` in shape: a small `TranscribeSummary`
// for the panel header + a heavy `PolyResult` (3-N notes with per-note
// pitch_bend_curve polylines) for the canvas-based PianoRoll. The Rust
// shell emits camelCase per the existing recording.ts convention; the
// IPC boundary (`transcribe_recording`, `get_poly_result`, `export_midi`,
// `transcribe-progress`) shares this shape with the Tier-5 Tauri-mock
// under tests/e2e/helpers.

import type { RecordingId } from "@/types/recording";

/** Single point in a per-note `pitch_bend_curve` polyline (`[tMs, cents]`). */
export interface BendPoint {
  readonly tMs: number;
  readonly cents: number;
}

/** Single note in a polyphonic transcription. MIDI 21..108 inclusive
 *  (88 keys). `velocity` is 0..127 as per Standard MIDI File semantics. */
export interface Note {
  readonly midi: number;
  readonly startMs: number;
  readonly durationMs: number;
  readonly velocity: number;
  readonly pitchBendCurve: readonly BendPoint[];
}

/** Lightweight summary returned by `transcribe_recording`. Carries the
 *  note count + duration the panel needs to render the complete branch
 *  without forcing a second IPC for the heavy `PolyResult`. `wasCached`
 *  drives the "Transcription cached" badge — same precedent as
 *  `AnalysisSummary.wasCached`. */
export interface TranscribeSummary {
  readonly recordingId: RecordingId;
  readonly noteCount: number;
  readonly durationMs: number;
  readonly wasCached: boolean;
  readonly transcriberVersion: string;
}

/** Full polyphonic transcription — fetched lazily after the summary lands. */
export interface PolyResult {
  readonly recordingId: RecordingId;
  readonly transcriberVersion: string;
  readonly durationMs: number;
  readonly notes: readonly Note[];
}

/** Streaming progress payload — emitted at ~10 Hz over the
 *  `transcribe-progress` Tauri channel while Basic Pitch / ONNX is running. */
export interface TranscribeProgress {
  readonly recordingId: RecordingId;
  readonly percent: number;
  readonly status?: "running" | "finalizing" | "failed";
  readonly error?: string;
}
