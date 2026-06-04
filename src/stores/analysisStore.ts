// Analysis store — slow-path Zustand state for the Phase 2.1 RecordingDetail.
//
// Per ADR-0003 the hot path (live PitchUpdate frames) lives in a RingBuffer,
// not Zustand. Analysis is intrinsically slow (one IPC per row click, one
// progress event per ~100 ms while a forced re-analyze is running) so the
// store is the right home for the summary card, contour cache, and
// in-progress / error sets.
//
// IPC surface (DESIGN.md §7.5 / §8.3):
//   - `analyze_recording({ recordingId, forceRefresh })` -> AnalysisSummary
//   - `get_contour({ recordingId })`                     -> ContourResult
//   - `analysis-progress` event channel emits AnalysisProgress at ~10 Hz
//
// Re-analyze flow:
//   1. `analyze(id, { forceRefresh: true })` adds the id to `inProgress`.
//   2. The IPC call is dispatched immediately, but the action does NOT
//      complete until an `analysis-progress` event arrives with
//      `percent >= 100` (or `status === "failed"`). This keeps the
//      <progress role="progressbar"> visible across the multi-tick
//      progression that the cache spec drives.
//   3. On completion the resolved summary is committed to `byRecording`,
//      `inProgress` is cleared, and a fresh `loadContour(id)` is
//      dispatched lazily so the static plot updates without blocking
//      the summary repaint.
//
// Cache flow:
//   1. `analyze(id)` (no `forceRefresh`) performs a cached read; the IPC
//      returns `was_cached=true` from the `analysis_cache` SHA-256 hit and
//      the summary lands in `byRecording` as soon as the promise resolves.
//
// Cross-references:
//   docs/design/DESIGN.md §7.5 (Phase 2.1 frontend additions)
//   docs/design/DESIGN.md §8.3 (analysis_cache schema)
//   src/stores/recordingsStore.ts (precedent for slow-path Zustand patterns)

import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { AnalysisProgress, AnalysisSummary, ContourResult } from "@/types/analysis";
import type { RecordingId } from "@/types/recording";

/** Snake_case wire format mirroring serde Rust output. The TS layer accepts
 *  either casing so the IPC boundary works for both the production Rust
 *  shell and the camelCase Tier-5 mock.
 *
 *  Production Rust shell (snake_case) sends `median_midi` / `median_cents_off`
 *  / `analyzer_version` and lacks `recording_id` (the front-end already knows
 *  the id it asked about). The Tier-5 mock sends camelCase `medianMidi` /
 *  `medianCents` / `analyzerVersion` / `recordingId`. `normaliseSummary`
 *  reads either flavour and falls through to `medianHzVoiced + a4Hz`-derived
 *  values when neither MIDI nor cents-off is present.
 */
interface WireSummary {
  recordingId?: string;
  recording_id?: string;
  // MIDI medians — Rust shell sends median_midi (Option<i32>); mock sends medianMidi.
  medianMidi?: number | null;
  median_midi?: number | null;
  // Cents-off medians — Rust shell sends median_cents_off (Option<f64>); mock sends medianCents.
  medianCents?: number | null;
  median_cents?: number | null;
  median_cents_off?: number | null;
  medianCentsOff?: number | null;
  // Median Hz — Rust shell sends median_hz_voiced (Option<f64>); mock omits it.
  medianHzVoiced?: number | null;
  median_hz_voiced?: number | null;
  voicedRatio?: number;
  voiced_ratio?: number;
  wasCached?: boolean;
  was_cached?: boolean;
  analyzerVersion?: string;
  analyzer_version?: string;
}

interface WireFrame {
  tMs?: number;
  t_ms?: number;
  centsFromMedian?: number;
  cents_from_median?: number;
  voiced?: boolean;
}

interface WireContour {
  recordingId?: string;
  recording_id?: string;
  analyzerVersion?: string;
  analyzer_version?: string;
  medianMidi?: number;
  median_midi?: number;
  medianCents?: number;
  median_cents?: number;
  voicedRatio?: number;
  voiced_ratio?: number;
  frames?: readonly WireFrame[];
}

/** Pick the first numerically-defined value from a list of optional fields.
 *  `null` is treated the same as `undefined` because Rust `Option<T>` round-
 *  trips to `null` over JSON (snake_case wire) but to `undefined` over the
 *  mock's structured-clone path. */
function firstNumber(...candidates: ReadonlyArray<number | null | undefined>): number | undefined {
  for (const v of candidates) {
    if (typeof v === "number" && Number.isFinite(v)) return v;
  }
  return undefined;
}

function normaliseSummary(raw: WireSummary): AnalysisSummary {
  // Prefer an explicit MIDI median (mock or future Rust); fall through to
  // deriving MIDI from `median_hz_voiced` when only Hz is supplied. The
  // production Rust shell at v0.2 emits BOTH `median_midi` and
  // `median_hz_voiced`, so the fall-through is just a defensive shim
  // until older shells are gone.
  const midiExplicit = firstNumber(raw.medianMidi, raw.median_midi);
  const medianHz = firstNumber(raw.medianHzVoiced, raw.median_hz_voiced);
  const medianMidi =
    midiExplicit !== undefined
      ? midiExplicit
      : medianHz !== undefined
        ? Math.round(69 + 12 * Math.log2(medianHz / 440))
        : 0;
  const medianCents = firstNumber(
    raw.medianCents,
    raw.median_cents,
    raw.medianCentsOff,
    raw.median_cents_off,
  );
  return {
    recordingId: raw.recordingId ?? raw.recording_id ?? "",
    medianMidi,
    medianCents: medianCents ?? 0,
    voicedRatio: raw.voicedRatio ?? raw.voiced_ratio ?? 0,
    wasCached: raw.wasCached ?? raw.was_cached ?? false,
    analyzerVersion: raw.analyzerVersion ?? raw.analyzer_version ?? "unknown",
  };
}

function normaliseContour(raw: WireContour): ContourResult {
  const frames = (raw.frames ?? []).map((f) => ({
    tMs: f.tMs ?? f.t_ms ?? 0,
    centsFromMedian: f.centsFromMedian ?? f.cents_from_median ?? 0,
    voiced: f.voiced ?? false,
  }));
  return {
    recordingId: raw.recordingId ?? raw.recording_id ?? "",
    analyzerVersion: raw.analyzerVersion ?? raw.analyzer_version ?? "unknown",
    medianMidi: raw.medianMidi ?? raw.median_midi ?? 0,
    medianCents: raw.medianCents ?? raw.median_cents ?? 0,
    voicedRatio: raw.voicedRatio ?? raw.voiced_ratio ?? 0,
    frames,
  };
}

/** Composite cache key matching the `${recId}:${analyzerVersion}` shape the
 *  Tier-5 mock seeds with. Two analyzer versions of the same recording
 *  coexist as separate entries until eviction. */
export function contourKey(recId: RecordingId, analyzerVersion: string): string {
  return `${recId}:${analyzerVersion}`;
}

interface AnalyzeOptions {
  readonly forceRefresh?: boolean;
}

export interface AnalysisState {
  /** Per-recording summary card payload. */
  byRecording: Map<RecordingId, AnalysisSummary>;
  /** Per-(recId, analyzerVersion) contour cache. */
  contoursByKey: Map<string, ContourResult>;
  /** Recording ids currently being analyzed. The AnalysisSummary card
   *  swaps the numeric readouts for a `<progress role="progressbar">`
   *  while the id is in this set. */
  inProgress: Set<RecordingId>;
  /** Per-recording in-flight progress percent (0..100). Drives the
   *  `<progress value=...>` attribute. */
  progressByRecording: Map<RecordingId, number>;
  /** Last error per recording id; the card surfaces a `role="alert"`
   *  when a key is present. */
  errors: Map<RecordingId, string>;
}

export interface AnalysisActions {
  analyze: (id: RecordingId, opts?: AnalyzeOptions) => Promise<void>;
  loadContour: (id: RecordingId) => Promise<void>;
  applyProgress: (p: AnalysisProgress) => void;
  /** Test-only helper to reset state between specs. Production code paths
   *  go through the IPC actions. */
  __resetForTest: () => void;
}

export type AnalysisStore = AnalysisState & AnalysisActions;

/** Per-id pending forced-refresh resolver. When `forceRefresh: true` is
 *  passed, `analyze` parks on a Promise that the next `analysis-progress`
 *  event with `percent >= 100` (or `status === "failed"`) resolves. This
 *  keeps the in-progress UI visible across multi-tick progressions
 *  without holding the IPC promise hostage. */
const forceRefreshResolvers = new Map<RecordingId, () => void>();

function shallowCopyMap<K, V>(m: Map<K, V>): Map<K, V> {
  return new Map(m);
}

function shallowCopySet<T>(s: Set<T>): Set<T> {
  return new Set(s);
}

export const useAnalysisStore = create<AnalysisStore>((set, get) => ({
  byRecording: new Map(),
  contoursByKey: new Map(),
  inProgress: new Set(),
  progressByRecording: new Map(),
  errors: new Map(),

  analyze: async (id, opts): Promise<void> => {
    const force = opts?.forceRefresh === true;
    // Mark in-progress before the IPC fires so the card's progress bar
    // becomes visible synchronously on click. The cache spec depends on
    // this ordering — `getByRole("progressbar")` runs immediately after
    // the re-analyze click resolves, before any awaits below.
    set((s) => {
      const inProgress = shallowCopySet(s.inProgress);
      inProgress.add(id);
      const progressByRecording = shallowCopyMap(s.progressByRecording);
      progressByRecording.set(id, 0);
      const errors = shallowCopyMap(s.errors);
      errors.delete(id);
      return { inProgress, progressByRecording, errors };
    });

    // For the forced-refresh path, we await both the IPC and a "100%"
    // progress event. The Tier-5 mock returns the summary synchronously,
    // but the spec drives the progress bar through 25 / 75 / 100 ticks
    // and asserts the bar disappears only after the final tick.
    const completion = force
      ? new Promise<void>((resolve) => {
          forceRefreshResolvers.set(id, resolve);
        })
      : Promise.resolve();

    try {
      const raw = await invoke<WireSummary>("analyze_recording", {
        recordingId: id,
        forceRefresh: force,
      });
      const summary = normaliseSummary(raw);
      // Wait for the 100% progress tick on the forced-refresh path.
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
      // Fire-and-forget: the contour fetch is large but the summary is
      // enough to render the header + card. Errors land in `errors` and
      // do not block the summary repaint.
      void get().loadContour(id);
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

  loadContour: async (id): Promise<void> => {
    try {
      const raw = await invoke<WireContour>("get_contour", { recordingId: id });
      const contour = normaliseContour(raw);
      set((s) => {
        const contoursByKey = shallowCopyMap(s.contoursByKey);
        contoursByKey.set(contourKey(id, contour.analyzerVersion), contour);
        return { contoursByKey };
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

  applyProgress: (p) => {
    set((s) => {
      const progressByRecording = shallowCopyMap(s.progressByRecording);
      progressByRecording.set(p.recordingId, p.percent);
      return { progressByRecording };
    });
    // 100% (or failed) terminates the forced-refresh await loop in
    // `analyze`. Resolving the parked Promise here keeps the IPC and the
    // progress channel decoupled — either source of truth can drive the
    // completion edge.
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
      contoursByKey: new Map(),
      inProgress: new Set(),
      progressByRecording: new Map(),
      errors: new Map(),
    }),
}));

/** Convenience selector: most-recent contour for a recording id, regardless
 *  of analyzer version. Matches what `installAnalysisMock`'s `get_contour`
 *  handler returns (first key whose prefix matches `${id}:`). */
export function selectLatestContour(
  state: AnalysisState,
  id: RecordingId,
): ContourResult | undefined {
  const prefix = `${id}:`;
  for (const [k, v] of state.contoursByKey) {
    if (k.startsWith(prefix)) return v;
  }
  return undefined;
}
