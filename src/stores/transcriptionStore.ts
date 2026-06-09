// Transcription store — slow-path Zustand state for the Phase 3
// TranscribePanel + PianoRoll.
//
// The hot path (canvas paint of the PolyResult) does NOT pass through
// Zustand: PianoRoll holds the result in a ref and drives its rAF loop
// from `playbackHeadRef` directly, mirroring how ContourLine consumes
// the playback head. Per-recording-open transitions (idle → in-progress
// → complete) are slow enough that a Zustand subscription is fine.
//
// IPC surface (mirrors the analysisStore precedent):
//   - `transcribe_recording({ recordingId, forceRefresh })` -> TranscribeSummary
//   - `get_poly_result({ recordingId })`                    -> PolyResult
//   - `export_midi({ recordingId, destPath })`              -> null
//   - `transcribe-progress` event channel emits TranscribeProgress at ~10 Hz
//
// Re-transcribe flow:
//   1. `transcribe(id, { forceRefresh: true })` adds the id to `inProgress`.
//   2. The IPC call dispatches immediately; the action does NOT complete
//      until a `transcribe-progress` event arrives with `percent >= 100`
//      (or `status === "failed"`). Same parking pattern as analysisStore's
//      `forceRefreshResolvers` — keeps the `<progress role="progressbar">`
//      visible across multi-tick progressions.
//   3. On completion the resolved summary is committed to `byRecording`,
//      `inProgress` is cleared, and a fresh `loadPolyResult(id)` fires
//      lazily so PianoRoll can paint without blocking the panel repaint.
//
//   src/stores/analysisStore.ts (precedent for the parked-promise pattern)

import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type {
  BendPoint,
  Note,
  PolyResult,
  TranscribeProgress,
  TranscribeSummary,
} from "@/types/transcription";
import type { RecordingId } from "@/types/recording";
import type { StemKind } from "@/types/stems";

/** Snake_case wire-format mirroring serde Rust output. The TS layer accepts
 *  either casing, exactly like analysisStore — the Tier-5 mock emits
 *  camelCase, the production Rust shell emits snake_case. */
interface WireSummary {
  recordingId?: string;
  recording_id?: string;
  noteCount?: number;
  note_count?: number;
  durationMs?: number;
  duration_ms?: number;
  wasCached?: boolean;
  was_cached?: boolean;
  transcriberVersion?: string;
  transcriber_version?: string;
}

interface WireBendPoint {
  tMs?: number;
  t_ms?: number;
  cents?: number;
}

interface WireNote {
  midi?: number;
  startMs?: number;
  start_ms?: number;
  durationMs?: number;
  duration_ms?: number;
  velocity?: number;
  pitchBendCurve?: readonly WireBendPoint[];
  pitch_bend_curve?: readonly WireBendPoint[];
}

interface WirePolyResult {
  recordingId?: string;
  recording_id?: string;
  transcriberVersion?: string;
  transcriber_version?: string;
  durationMs?: number;
  duration_ms?: number;
  notes?: readonly WireNote[];
}

interface WireProgress {
  recordingId?: string;
  recording_id?: string;
  percent?: number;
  status?: "running" | "finalizing" | "failed";
  error?: string;
}

function normaliseSummary(raw: WireSummary): TranscribeSummary {
  return {
    recordingId: raw.recordingId ?? raw.recording_id ?? "",
    noteCount: raw.noteCount ?? raw.note_count ?? 0,
    durationMs: raw.durationMs ?? raw.duration_ms ?? 0,
    wasCached: raw.wasCached ?? raw.was_cached ?? false,
    transcriberVersion: raw.transcriberVersion ?? raw.transcriber_version ?? "unknown",
  };
}

function normaliseBend(raw: WireBendPoint): BendPoint {
  return {
    tMs: raw.tMs ?? raw.t_ms ?? 0,
    cents: typeof raw.cents === "number" ? raw.cents : 0,
  };
}

function normaliseNote(raw: WireNote): Note {
  const bendRaw = raw.pitchBendCurve ?? raw.pitch_bend_curve ?? [];
  return {
    midi: raw.midi ?? 0,
    startMs: raw.startMs ?? raw.start_ms ?? 0,
    durationMs: raw.durationMs ?? raw.duration_ms ?? 0,
    velocity: raw.velocity ?? 0,
    pitchBendCurve: bendRaw.map(normaliseBend),
  };
}

function normalisePolyResult(raw: WirePolyResult): PolyResult {
  return {
    recordingId: raw.recordingId ?? raw.recording_id ?? "",
    transcriberVersion: raw.transcriberVersion ?? raw.transcriber_version ?? "unknown",
    durationMs: raw.durationMs ?? raw.duration_ms ?? 0,
    notes: (raw.notes ?? []).map(normaliseNote),
  };
}

function normaliseProgress(raw: WireProgress): TranscribeProgress {
  const recordingId: RecordingId = raw.recordingId ?? raw.recording_id ?? "";
  const percent = raw.percent ?? 0;
  const base: { recordingId: RecordingId; percent: number } = { recordingId, percent };
  const withStatus = raw.status !== undefined ? { ...base, status: raw.status } : base;
  return raw.error !== undefined ? { ...withStatus, error: raw.error } : withStatus;
}

export { normaliseProgress as __normaliseTranscribeProgress };

/** Composite cache key matching the `${recId}:${transcriberVersion}` shape
 *  the Tier-5 mock seeds with. Mirrors `contourKey` in analysisStore. */
export function polyResultKey(recId: RecordingId, transcriberVersion: string): string {
  return `${recId}:${transcriberVersion}`;
}

interface TranscribeOptions {
  readonly forceRefresh?: boolean;
  /** Phase 5: optional stem kind. When set, the IPC payload carries
   *  `stemKind` so the Rust shell transcribes the stem FLAC instead of
   *  the original mix. The cache key gains `stemKind` so the four stems
   *  and the mix do not clobber each other. */
  readonly stemKind?: StemKind;
}

export interface TranscriptionState {
  /** Per-recording summary panel payload. */
  byRecording: Map<RecordingId, TranscribeSummary>;
  /** Per-(recId, transcriberVersion) PolyResult cache. */
  polyResultsByKey: Map<string, PolyResult>;
  /** Recording ids currently being transcribed. The TranscribePanel swaps
   *  the idle button for `<progress role="progressbar">` while the id is
   *  in this set. */
  inProgress: Set<RecordingId>;
  /** Per-recording in-flight progress percent (0..100). Drives the
   *  `<progress value=...>` attribute. */
  progressByRecording: Map<RecordingId, number>;
  /** Last error per recording id; the panel surfaces a `role="alert"`
   *  when a key is present. */
  errors: Map<RecordingId, string>;
}

export interface TranscriptionActions {
  transcribe: (id: RecordingId, opts?: TranscribeOptions) => Promise<void>;
  loadPolyResult: (id: RecordingId) => Promise<void>;
  exportMidi: (id: RecordingId, destPath: string) => Promise<void>;
  applyProgress: (p: TranscribeProgress) => void;
  /** Test-only helper to reset state between specs. */
  __resetForTest: () => void;
}

export type TranscriptionStore = TranscriptionState & TranscriptionActions;

/** Per-id pending forced-refresh resolver. Same pattern as analysisStore:
 *  `transcribe(id, { forceRefresh: true })` parks on a Promise that the
 *  next progress event with `percent >= 100` (or `status === "failed"`)
 *  resolves. Decouples the IPC and progress channels. */
const forceRefreshResolvers = new Map<RecordingId, () => void>();

function shallowCopyMap<K, V>(m: Map<K, V>): Map<K, V> {
  return new Map(m);
}

function shallowCopySet<T>(s: Set<T>): Set<T> {
  return new Set(s);
}

export const useTranscriptionStore = create<TranscriptionStore>((set, get) => ({
  byRecording: new Map(),
  polyResultsByKey: new Map(),
  inProgress: new Set(),
  progressByRecording: new Map(),
  errors: new Map(),

  transcribe: async (id, opts): Promise<void> => {
    const force = opts?.forceRefresh === true;
    const stemKind = opts?.stemKind;
    // Mark in-progress before the IPC fires so the panel's progress bar
    // becomes visible synchronously on click. The button spec depends on
    // this ordering — the progress bar must surface before any awaits.
    set((s) => {
      const inProgress = shallowCopySet(s.inProgress);
      inProgress.add(id);
      const progressByRecording = shallowCopyMap(s.progressByRecording);
      progressByRecording.set(id, 0);
      const errors = shallowCopyMap(s.errors);
      errors.delete(id);
      return { inProgress, progressByRecording, errors };
    });

    // Wait for the 100% progress tick on every path (cached and forced)
    // so the bar stays visible across the multi-tick progression the
    // spec drives via `pushTranscribeProgress`. The mock returns the
    // summary synchronously, so the IPC promise resolves before the
    // first push — without the parked completion the bar would flicker
    // off-then-on as the ticks land.
    const completion = new Promise<void>((resolve) => {
      forceRefreshResolvers.set(id, resolve);
    });

    try {
      const ipcArgs: Record<string, unknown> = {
        recordingId: id,
        forceRefresh: force,
      };
      if (stemKind !== undefined) ipcArgs["stemKind"] = stemKind;
      const raw = await invoke<WireSummary>("transcribe_recording", ipcArgs);
      const summary = normaliseSummary(raw);
      // Park on the completion edge — either a real progress event with
      // `percent >= 100` or the failure path drives this.
      await completion;
      set((s) => {
        const byRecording = shallowCopyMap(s.byRecording);
        byRecording.set(id, summary);
        const inProgress = shallowCopySet(s.inProgress);
        inProgress.delete(id);
        const progressByRecording = shallowCopyMap(s.progressByRecording);
        progressByRecording.delete(id);
        return { byRecording, inProgress, progressByRecording };
      });
      forceRefreshResolvers.delete(id);
      // Fire-and-forget: PolyResult is heavy but the summary alone is
      // enough to render the complete branch ("Notes detected: N").
      void get().loadPolyResult(id);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      forceRefreshResolvers.delete(id);
      set((s) => {
        const inProgress = shallowCopySet(s.inProgress);
        inProgress.delete(id);
        const progressByRecording = shallowCopyMap(s.progressByRecording);
        progressByRecording.delete(id);
        const errors = shallowCopyMap(s.errors);
        errors.set(id, msg);
        return { inProgress, progressByRecording, errors };
      });
    }
  },

  loadPolyResult: async (id): Promise<void> => {
    try {
      const raw = await invoke<WirePolyResult>("get_poly_result", { recordingId: id });
      const poly = normalisePolyResult(raw);
      set((s) => {
        const polyResultsByKey = shallowCopyMap(s.polyResultsByKey);
        polyResultsByKey.set(polyResultKey(id, poly.transcriberVersion), poly);
        return { polyResultsByKey };
      });
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      set((s) => {
        const errors = shallowCopyMap(s.errors);
        errors.set(id, msg);
        return { errors };
      });
    }
  },

  exportMidi: async (id, destPath): Promise<void> => {
    try {
      await invoke<null>("export_midi", { recordingId: id, destPath });
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      set((s) => {
        const errors = shallowCopyMap(s.errors);
        errors.set(id, msg);
        return { errors };
      });
    }
  },

  applyProgress: (p) => {
    set((s) => {
      const progressByRecording = shallowCopyMap(s.progressByRecording);
      progressByRecording.set(p.recordingId, p.percent);
      return { progressByRecording };
    });
    // 100% (or failed) terminates the parked completion in `transcribe`.
    if (p.percent >= 100 || p.status === "failed") {
      const resolver = forceRefreshResolvers.get(p.recordingId);
      if (resolver !== undefined) {
        forceRefreshResolvers.delete(p.recordingId);
        resolver();
      }
    }
  },

  __resetForTest: () =>
    set({
      byRecording: new Map(),
      polyResultsByKey: new Map(),
      inProgress: new Set(),
      progressByRecording: new Map(),
      errors: new Map(),
    }),
}));

/** Convenience selector: most-recent PolyResult for a recording id,
 *  regardless of transcriber version. Mirrors `selectLatestContour`. */
export function selectLatestPolyResult(
  state: TranscriptionState,
  id: RecordingId,
): PolyResult | undefined {
  const prefix = `${id}:`;
  for (const [k, v] of state.polyResultsByKey) {
    if (k.startsWith(prefix)) return v;
  }
  return undefined;
}
