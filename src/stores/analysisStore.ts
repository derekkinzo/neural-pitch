// Analysis store — slow-path Zustand state for the RecordingDetail panel.
//
// The hot path (live PitchUpdate frames) lives in a RingBuffer,
// not Zustand. Analysis is intrinsically slow (one IPC per row click, one
// progress event per ~100 ms while a forced re-analyze is running) so the
// store is the right home for the summary card, contour cache, and
// in-progress / error sets.
//
// IPC surface:
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
// See also `src/stores/recordingsStore.ts` for the same slow-path
// Zustand store pattern.

import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type {
  AnalysisProgress,
  AnalysisSummary,
  ContourResult,
  RangeReport,
  VibratoReport,
  VibratoWindow,
} from "@/types/analysis";
import type { RecordingId } from "@/types/recording";

/** Snake_case wire format mirroring serde Rust output. The TS layer accepts
 *  either casing so the IPC boundary works for both the production Rust
 *  shell and the camelCase E2E mock.
 *
 *  Production Rust shell (snake_case) sends `median_midi` / `median_cents_off`
 *  / `analyzer_version` and lacks `recording_id` (the front-end already knows
 *  the id it asked about). The E2E mock sends camelCase `medianMidi` /
 *  `medianCents` / `analyzerVersion` / `recordingId`. `normaliseSummary`
 *  reads either flavour and falls through to `medianHzVoiced + a4Hz`-derived
 *  values when neither MIDI nor cents-off is present.
 */
interface WireRangeReport {
  comfortableLowMidi?: number;
  comfortable_low_midi?: number;
  comfortableHighMidi?: number;
  comfortable_high_midi?: number;
  fullLowMidi?: number;
  full_low_midi?: number;
  fullHighMidi?: number;
  full_high_midi?: number;
  voicedFrameCount?: number;
  voiced_frame_count?: number;
  voiceTypeHints?: readonly string[];
  voice_type_hints?: readonly string[];
}

interface WireVibratoWindow {
  tMs?: number;
  t_ms?: number;
  rateHz?: number;
  rate_hz?: number;
  extentCents?: number;
  extent_cents?: number;
  confidence?: number;
}

interface WireVibratoReport {
  medianRateHz?: number;
  median_rate_hz?: number;
  medianExtentCents?: number;
  median_extent_cents?: number;
  vibratoRatio?: number;
  vibrato_ratio?: number;
  windows?: readonly WireVibratoWindow[];
}

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
  // Range / vibrato — both casings accepted; either is forwarded into the
  // already-existing `byRecording` Map entry, so RangeReadout /
  // VibratoReadout read from the same store entry as the summary card.
  range?: WireRangeReport | null;
  range_report?: WireRangeReport | null;
  vibrato?: WireVibratoReport | null;
  vibrato_report?: WireVibratoReport | null;
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

function normaliseRange(raw: WireRangeReport | null | undefined): RangeReport | undefined {
  if (raw === null || raw === undefined) return undefined;
  const comfortableLow = firstNumber(raw.comfortableLowMidi, raw.comfortable_low_midi);
  const comfortableHigh = firstNumber(raw.comfortableHighMidi, raw.comfortable_high_midi);
  const fullLow = firstNumber(raw.fullLowMidi, raw.full_low_midi);
  const fullHigh = firstNumber(raw.fullHighMidi, raw.full_high_midi);
  const voicedFrames = firstNumber(raw.voicedFrameCount, raw.voiced_frame_count);
  const hints = raw.voiceTypeHints ?? raw.voice_type_hints ?? [];
  return {
    comfortableLowMidi: comfortableLow ?? 0,
    comfortableHighMidi: comfortableHigh ?? 0,
    fullLowMidi: fullLow ?? 0,
    fullHighMidi: fullHigh ?? 0,
    voicedFrameCount: voicedFrames ?? 0,
    voiceTypeHints: [...hints],
  };
}

function normaliseVibrato(raw: WireVibratoReport | null | undefined): VibratoReport | undefined {
  if (raw === null || raw === undefined) return undefined;
  const medianRate = firstNumber(raw.medianRateHz, raw.median_rate_hz);
  const medianExtent = firstNumber(raw.medianExtentCents, raw.median_extent_cents);
  const ratio = firstNumber(raw.vibratoRatio, raw.vibrato_ratio);
  const windows: VibratoWindow[] = (raw.windows ?? []).map((w) => ({
    tMs: firstNumber(w.tMs, w.t_ms) ?? 0,
    rateHz: firstNumber(w.rateHz, w.rate_hz) ?? 0,
    extentCents: firstNumber(w.extentCents, w.extent_cents) ?? 0,
    confidence: typeof w.confidence === "number" ? w.confidence : 0,
  }));
  return {
    medianRateHz: medianRate ?? 0,
    medianExtentCents: medianExtent ?? 0,
    vibratoRatio: ratio ?? 0,
    windows,
  };
}

function normaliseSummary(raw: WireSummary): AnalysisSummary {
  // Both `median_midi` and `median_hz_voiced` are accepted; the Hz
  // fall-through covers shells that emit only Hz (e.g. the camelCase
  // E2E mock or callers that pre-date the explicit MIDI field).
  const midiExplicit = firstNumber(raw.medianMidi, raw.median_midi);
  const medianHz = firstNumber(raw.medianHzVoiced, raw.median_hz_voiced);
  // `medianHz > 0` guard is required: `Math.log2(0) === -Infinity`, so a
  // shim or mock that emits literal `0` rather than the documented `null`
  // sentinel for unvoiced takes would otherwise propagate `-Infinity`
  // into `medianMidi` and from there into `formatMidiNote`.
  const medianMidi =
    midiExplicit !== undefined
      ? midiExplicit
      : medianHz !== undefined && medianHz > 0
        ? Math.round(69 + 12 * Math.log2(medianHz / 440))
        : 0;
  const medianCents = firstNumber(
    raw.medianCents,
    raw.median_cents,
    raw.medianCentsOff,
    raw.median_cents_off,
  );
  const range = normaliseRange(raw.range ?? raw.range_report);
  const vibrato = normaliseVibrato(raw.vibrato ?? raw.vibrato_report);
  // `exactOptionalPropertyTypes` rejects `range: undefined` literals on a
  // `range?: RangeReport` field. Emit the keys only when defined so the
  // resulting AnalysisSummary stays compatible with the readonly optional
  // shape declared in `src/types/analysis.ts`.
  return {
    recordingId: raw.recordingId ?? raw.recording_id ?? "",
    medianMidi,
    medianCents: medianCents ?? 0,
    voicedRatio: raw.voicedRatio ?? raw.voiced_ratio ?? 0,
    wasCached: raw.wasCached ?? raw.was_cached ?? false,
    analyzerVersion: raw.analyzerVersion ?? raw.analyzer_version ?? "unknown",
    ...(range !== undefined ? { range } : {}),
    ...(vibrato !== undefined ? { vibrato } : {}),
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
 *  E2E mock seeds with. Two analyzer versions of the same recording
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
    // progress event. The E2E mock returns the summary synchronously,
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
