// Recording — wire-format and progress event types for the Phase 2.0 recorder.
//
// Field names are camelCase on the TS side; the IPC boundary maps from
// snake_case Rust per the existing `audio-event.ts` convention.
//

/** UUIDv7 string identifier minted on `start_recording` and persisted in
 *  the SQLite library. */
export type RecordingId = string;

/** Persisted metadata for a single take. Mirrors the row layout in §8.3
 *  with `createdAt` already converted from ISO-8601 to ms-epoch. */
export interface Recording {
  readonly id: RecordingId;
  readonly filename: string;
  readonly createdAt: number;
  readonly durationMs: number;
  readonly sampleRateHz: number;
  readonly channels: number;
  readonly bitDepth: number;
  readonly a4Hz: number;
  readonly instrumentProfile: string;
  readonly userLabel?: string;
}

/** Streaming progress payload — emitted at ~5 Hz over the
 *  `recording-progress` Tauri channel while a take is active. */
export interface RecordingProgress {
  readonly recordingId: RecordingId;
  readonly elapsedMs: number;
  readonly sampleCount: number;
  readonly droppedWindows: number;
  readonly status: "active" | "finalizing" | "failed";
  readonly error?: string;
}
