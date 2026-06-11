// Stems — wire-format types for the HTDemucs source separation
// subsystem.
//
// Mirrors `src/types/transcription.ts` in shape: a finite per-recording
// state (`SeparationStatus`) drives the panel; the heavy work
// (`Channel<SeparateProgress>` + four FLACs on disk) lives outside the
// store. Field names are camelCase on the TS side; the Rust IPC boundary
// maps from snake_case per the existing `transcription.ts` convention.

import type { RecordingId } from "@/types/recording";

/** The four standard Demucs stems. Order is fixed across the UI: vocals
 *  → drums → bass → other. */
export type StemKind = "vocals" | "drums" | "bass" | "other";

/** Stage label published per progress frame. The four stem kinds plus
 *  "finalizing" (FLAC encode + mux) round-trip the cycle. */
export type SeparateStage = StemKind | "finalizing";

/** Finite-state machine governing the StemSeparationPanel render branch. */
export type SeparationStatus = "idle" | "downloading-model" | "separating" | "complete" | "error";

/** Per-recording state slot held in the stems store. */
export interface StemSeparationState {
  readonly status: SeparationStatus;
  readonly stemPaths?: Readonly<Record<StemKind, string>>;
  readonly error?: string;
}

/** Streaming progress payload — emitted at ~10–20 Hz over the
 *  `separate-progress` Tauri channel while HTDemucs is running. */
export interface SeparateProgress {
  readonly recordingId: RecordingId;
  readonly stage: SeparateStage;
  readonly percent: number;
}

/** Static metadata about the bundled HTDemucs model. Surfaces the
 *  download URL + checksum so the offline-first-run error branch can
 *  show a copyable URL the user can fetch manually. */
export interface StemModelInfo {
  readonly downloadUrl: string;
  readonly sha256: string;
  readonly sizeBytes: number;
}

/** Fixed display order — the four-card grid renders in this sequence. */
export const STEM_KIND_ORDER: ReadonlyArray<StemKind> = ["vocals", "drums", "bass", "other"];

/** Title-case display label for a stem kind. The keys in `STEM_KIND_ORDER`
 *  match the wire format exactly so the lookup is total. */
export const STEM_DISPLAY_LABEL: Readonly<Record<StemKind, string>> = {
  vocals: "Vocals",
  drums: "Drums",
  bass: "Bass",
  other: "Other",
};
