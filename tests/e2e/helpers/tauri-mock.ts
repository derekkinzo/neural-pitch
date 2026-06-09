// Tauri IPC mock bridge for Playwright.
//
//
// Implementation note: rather than dynamically importing
// `@tauri-apps/api/mocks` from the init script (which is async and races
// against the React `useEffect` that calls `invoke('greet')`), we inline the
// minimal synchronous shape of `mockIPC` directly. This mirrors what the
// upstream `mockIPC` function does — sets `window.__TAURI_INTERNALS__.invoke`
// plus the callback-registry fields — but does it before any application
// script runs, so the very first `invoke()` call sees the mock.
//
// Phase 1.2 extends the bridge with:
//   - default responses for `start_capture`, `stop_capture`, `configure`
//   - a synthetic Channel<PitchUpdate> path: the page-side `usePitchStream`
//     hook registers itself on `__neuralPitchTestHooks.listeners` and tests
//     drive frames via `pushPitchUpdate(page, snapshot)`.

import type { Page } from "@playwright/test";

/**
 * Per-command response. A function form receives the raw IPC payload
 * (the `args` object passed to `invoke`) and returns either the response
 * value or a Promise of it. Functions are serialised to source via
 * `Function.prototype.toString` and re-evaluated in the page context.
 */
export type TauriMockResponse =
  | unknown
  | ((args: Record<string, unknown>) => unknown | Promise<unknown>);

export type TauriMockResponses = Record<string, TauriMockResponse>;

/**
 * A serialisable description of a single mock entry: either a literal value
 * or a function-source string that the page-side init script will evaluate.
 */
interface SerialisedEntry {
  kind: "value" | "function";
  body: unknown;
}

/**
 * Phase-1.2 default responses. Adding entries here is how new Tauri commands
 * acquire a mock baseline that all specs share.
 */
export const defaultResponses: TauriMockResponses = {
  greet: (args: Record<string, unknown>) => {
    const name = typeof args["name"] === "string" ? (args["name"] as string) : "world";
    return `Hello, ${name}! NeuralPitch core says hi.`;
  },
  start_capture: () => ({
    device_name: "Mock Microphone",
    sample_rate_hz: 48000,
    window_samples: 2048,
    hop_samples: 512,
  }),
  stop_capture: null,
  configure: null,
  get_audio_params: () => ({
    sample_rate_hz: 48000,
    window_samples: 2048,
    hop_samples: 512,
  }),
  // Phase 2.0: PlaybackPanel resolves the on-disk path when a row is
  // selected. Specs that exercise the detail panel (Phase 2.1) inherit a
  // benign placeholder so the panel can mount without each spec re-seeding.
  get_recording_path: (args: Record<string, unknown>) => {
    const id = typeof args["id"] === "string" ? (args["id"] as string) : "rec";
    return `/tmp/${id}.flac`;
  },
  // Phase 2.1: baseline analyze + contour responders so specs that only
  // exercise the recordings list (not the detail panel) inherit non-throwing
  // defaults. Per-spec installAnalysisMock(...) overrides this baseline with
  // seeded summaries / contours.
  analyze_recording: (args: Record<string, unknown>) => {
    const recordingId =
      typeof args["recordingId"] === "string" ? (args["recordingId"] as string) : "rec";
    const force = Boolean(args["forceRefresh"]);
    return {
      recordingId,
      medianMidi: 69,
      medianCents: 0,
      voicedRatio: 0,
      wasCached: !force,
      analyzerVersion: "pyin-0.1.0",
    };
  },
  get_contour: (args: Record<string, unknown>) => {
    const recordingId =
      typeof args["recordingId"] === "string" ? (args["recordingId"] as string) : "rec";
    return {
      recordingId,
      analyzerVersion: "pyin-0.1.0",
      medianMidi: 69,
      medianCents: 0,
      voicedRatio: 0,
      frames: [],
    };
  },
};

function serialise(responses: TauriMockResponses): Record<string, SerialisedEntry> {
  const out: Record<string, SerialisedEntry> = {};
  for (const [k, v] of Object.entries(responses)) {
    if (typeof v === "function") {
      out[k] = { kind: "function", body: v.toString() };
    } else {
      out[k] = { kind: "value", body: v };
    }
  }
  return out;
}

/**
 * Install the mock-IPC bridge into the page before any script runs.
 *
 * The init script:
 *   1. Sets `window.__E2E__ = true` as a runtime sentinel.
 *   2. Stashes the response map on `window.__neuralPitchTestHooks` so
 *      `pushPitchUpdate` (below) and per-spec overrides can mutate it.
 *   3. Synchronously installs `window.__TAURI_INTERNALS__.invoke` to route
 *      every `invoke()` call through the response map.
 *   4. Tracks invoke call counts on `__neuralPitchTestHooks.invokeCalls` so
 *      specs can assert e.g. exactly-one `configure` per A4 change.
 */
export async function installTauriMock(
  page: Page,
  responses: TauriMockResponses = {},
): Promise<void> {
  const merged: TauriMockResponses = { ...defaultResponses, ...responses };
  const serialised = serialise(merged);

  await page.addInitScript((entries: Record<string, SerialisedEntry>) => {
    type Handler = (args: Record<string, unknown>) => unknown | Promise<unknown>;
    type Internals = {
      invoke?: (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
      transformCallback?: (cb?: (data: unknown) => void, once?: boolean) => number;
      unregisterCallback?: (id: number) => void;
      runCallback?: (id: number, data: unknown) => void;
      callbacks?: Map<number, (data: unknown) => void>;
      metadata?: {
        currentWindow: { label: string };
        currentWebview: { windowLabel: string; label: string };
      };
    };
    // Allow the merge path below to preserve fields seeded by other init
    // scripts (e.g. `installPlaybackRoutes`'s `convertFileSrc`). Index
    // signature for unknown extras keeps them through the spread.
    type Hooks = {
      handlers: Map<string, Handler | unknown>;
      listeners: Map<string, Array<(payload: unknown) => void>>;
      invokeCalls: Array<{ cmd: string; args: Record<string, unknown> }>;
      [extra: string]: unknown;
    };
    type WindowWithHooks = Window & {
      __E2E__?: boolean;
      __TAURI_INTERNALS__?: Internals;
      __neuralPitchTestHooks?: Partial<Hooks>;
    };
    const w = window as WindowWithHooks;
    w.__E2E__ = true;

    const handlers = new Map<string, Handler | unknown>();
    for (const [cmd, entry] of Object.entries(entries)) {
      if (entry.kind === "function") {
        // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
        const fn = new Function("return (" + (entry.body as string) + ")")() as Handler;
        handlers.set(cmd, fn);
      } else {
        handlers.set(cmd, entry.body);
      }
    }

    // Merge into any pre-existing hook object so init-script registration
    // order does not silently lose fields (e.g. `convertFileSrc` seeded by
    // `installPlaybackRoutes` if it runs before this script). Replacing
    // wholesale would clobber the resolver and yield a hard-to-debug
    // asset 404 in CI.
    const existing: Partial<Hooks> = w.__neuralPitchTestHooks ?? {};
    w.__neuralPitchTestHooks = {
      ...existing,
      handlers,
      listeners: existing.listeners ?? new Map(),
      invokeCalls: existing.invokeCalls ?? [],
    };

    const internals: Internals = w.__TAURI_INTERNALS__ ?? {};
    const callbacks = new Map<number, (data: unknown) => void>();
    internals.callbacks = callbacks;
    internals.transformCallback = (cb, once) => {
      const id = Math.floor(Math.random() * 0xffffffff);
      callbacks.set(id, (data) => {
        if (once === true) callbacks.delete(id);
        if (cb) cb(data);
      });
      return id;
    };
    internals.unregisterCallback = (id) => {
      callbacks.delete(id);
    };
    internals.runCallback = (id, data) => {
      const fn = callbacks.get(id);
      if (fn) fn(data);
    };
    internals.invoke = async (cmd, args) => {
      const a = args ?? {};
      w.__neuralPitchTestHooks?.invokeCalls?.push({ cmd, args: a });
      const handler = handlers.get(cmd);
      if (handler === undefined) {
        throw new Error(`unmocked Tauri command: ${cmd}`);
      }
      if (typeof handler === "function") {
        return await (handler as Handler)(a);
      }
      return handler;
    };
    internals.metadata = {
      currentWindow: { label: "main" },
      currentWebview: { windowLabel: "main", label: "main" },
    };
    w.__TAURI_INTERNALS__ = internals;
  }, serialised);
}

/**
 * Snake_case PitchUpdate as serialised by the Rust pipeline.
 * Mirrors `crates/neural-pitch-core/src/pipeline/sink.rs::PitchUpdate`.
 */
export interface MockPitchUpdate {
  timestamp_samples: number;
  f0_hz: number;
  confidence: number;
  voiced: boolean;
  smoothed_cents: number;
  target_midi: number;
  target_hz: number;
}

/**
 * Push a simulated PitchUpdate frame through the synthetic Channel.
 *
 * The page-side `usePitchStream` hook registers itself on
 * `window.__neuralPitchTestHooks.listeners` for the `"pitch-update"` event;
 * this helper walks that list and delivers the payload to each listener,
 * which forwards into the same code path as a real `runCallback` from the
 * Rust shell.
 */
export async function pushPitchUpdate(page: Page, update: MockPitchUpdate): Promise<void> {
  await page.evaluate((frame) => {
    type WindowWithHooks = Window & {
      __neuralPitchTestHooks?: {
        listeners: Map<string, Array<(payload: unknown) => void>>;
      };
    };
    const w = window as WindowWithHooks;
    const listeners = w.__neuralPitchTestHooks?.listeners.get("pitch-update") ?? [];
    for (const fn of listeners) {
      fn(frame);
    }
  }, update);
}

/**
 * Wire-format AudioBackendEvent variants the Tier-5 specs synthesise.
 * Mirrors the Phase-1.3 IPC contract.
 */
export type MockAudioBackendEvent =
  | { type: "PriorNarrowed"; rangeHz: readonly [number, number] }
  | { type: "Disconnected" }
  | { type: "Connected"; rateHz?: number; channels?: number }
  | { type: "FormatChanged"; rateHz: number; channels: number };

/**
 * Push a synthetic `audio:backend` event through the test bridge. The
 * page-side `useDeviceEvents` hook registers a listener on
 * `__neuralPitchTestHooks.listeners.get("audio:backend")` and routes the
 * payload into `tunerStore` — exactly what the Rust shell does in
 * production.
 */
export async function pushDeviceEvent(page: Page, event: MockAudioBackendEvent): Promise<void> {
  await page.evaluate((payload) => {
    type WindowWithHooks = Window & {
      __neuralPitchTestHooks?: {
        listeners: Map<string, Array<(payload: unknown) => void>>;
      };
    };
    const w = window as WindowWithHooks;
    const listeners = w.__neuralPitchTestHooks?.listeners.get("audio:backend") ?? [];
    for (const fn of listeners) {
      fn(payload);
    }
  }, event);
}

/**
 * Helper to construct a self-consistent `MockPitchUpdate` from a frequency
 * and a cents deviation. Tests describe the world in musical terms
 * (`A4 + 0¢`) and the helper computes `target_midi`, `target_hz`, etc.
 */
export function makePitchUpdate(opts: {
  f0Hz: number;
  cents?: number;
  voiced?: boolean;
  confidence?: number;
  a4Hz?: number;
}): MockPitchUpdate {
  const a4 = opts.a4Hz ?? 440;
  const voiced = opts.voiced ?? true;
  const cents = opts.cents ?? 0;
  const midiFloat = 69 + 12 * Math.log2(opts.f0Hz / a4);
  const targetMidi = Math.round(midiFloat);
  const targetHz = a4 * Math.pow(2, (targetMidi - 69) / 12);
  return {
    timestamp_samples: 0,
    f0_hz: opts.f0Hz,
    confidence: opts.confidence ?? 0.95,
    voiced,
    smoothed_cents: cents,
    target_midi: targetMidi,
    target_hz: targetHz,
  };
}

/**
 * Phase-2.0 recordings — wire-format mirrors `src/types/recording.ts` (planned).
 * Field names are camelCase on the TS side; the Rust IPC boundary maps from
 * snake_case per the audio-event.ts convention. We keep the test-side type
 * camelCase so specs read in the same vocabulary as the React components
 * they exercise.
 */
export interface MockRecording {
  id: string;
  filename: string;
  createdAt: number;
  durationMs: number;
  sampleRateHz: number;
  channels: number;
  bitDepth: number;
  a4Hz: number;
  instrumentProfile: string;
  userLabel?: string;
}

export interface MockRecordingProgress {
  recordingId: string;
  elapsedMs: number;
  sampleCount: number;
  droppedWindows: number;
  status: "active" | "finalizing" | "failed";
  error?: string;
}

/**
 * Install mock responses for the Phase-2.0 recordings IPC surface.
 *
 * The handlers mutate a shared list stashed on `window.__neuralPitchTestHooks
 * .recordings`. The seed array is JSON-encoded into the handler source so
 * the page-side copy is self-contained — closures do not survive
 * `Function.prototype.toString()`, so each handler self-initialises the
 * shared slot from its embedded seed on first call.
 *
 * Phase-2.0 contract:
 *   - `list_recordings()`                          → MockRecording[]
 *     (returned descending by createdAt to match the store selector)
 *   - `start_recording({ instrumentProfile })`     → { recordingId }
 *   - `stop_recording()`                           → MockRecording
 *   - `delete_recording({ id })`                   → null
 *   - `rename_recording({ id, label })`            → null
 *
 * Specs combine this with the base mock map:
 *
 *     await mockTauri.install({ ...installRecordingsMock(seed) });
 */
export function installRecordingsMock(records: MockRecording[]): TauriMockResponses {
  // JSON-encode once. We inline the literal into each handler source via
  // template substitution at .toString() time: every handler starts with
  //   const __seed__ = JSON.parse('<encoded>');
  // and serialise() captures that literal directly because it lives in the
  // function body, not in a closure.
  const seedJson = JSON.stringify(records);

  // Build the handler bodies as strings, then parse via `new Function` so
  // serialise() picks up genuine Function objects (its `typeof === "function"`
  // branch). This sidesteps the closure-capture problem: every reference to
  // the seed is re-emitted as a literal in the function body each time.
  const seedHydrate = `
    var w = window;
    var hooks = w.__neuralPitchTestHooks || {};
    if (!hooks.recordings) {
      hooks.recordings = JSON.parse(${JSON.stringify(seedJson)});
    }
    w.__neuralPitchTestHooks = hooks;
  `;

  // Each handler is a Function that takes `args` and returns a value or
  // promise. We use `new Function` so the body is parseable JS and the
  // serialise() pipeline picks up the literal source verbatim.
  // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
  const listHandler = new Function(
    "args",
    `${seedHydrate}
     var list = window.__neuralPitchTestHooks.recordings || [];
     return list.slice().sort(function (a, b) { return b.createdAt - a.createdAt; });`,
  ) as (args: Record<string, unknown>) => unknown;

  // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
  const startHandler = new Function(
    "args",
    `${seedHydrate}
     var id = "rec-" + Date.now() + "-" + Math.floor(Math.random() * 1e6).toString(16);
     window.__neuralPitchTestHooks.activeRecordingId = id;
     window.__neuralPitchTestHooks.lastStartArgs = Object.assign({}, args);
     return { recordingId: id };`,
  ) as (args: Record<string, unknown>) => unknown;

  // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
  const stopHandler = new Function(
    "args",
    `${seedHydrate}
     var id = window.__neuralPitchTestHooks.activeRecordingId || ("rec-" + Date.now());
     var rec = {
       id: id,
       filename: id + ".flac",
       createdAt: Date.now(),
       durationMs: 1230,
       sampleRateHz: 48000,
       channels: 1,
       bitDepth: 24,
       a4Hz: 440,
       instrumentProfile: "Voice"
     };
     window.__neuralPitchTestHooks.recordings = [rec].concat(window.__neuralPitchTestHooks.recordings || []);
     window.__neuralPitchTestHooks.activeRecordingId = undefined;
     return rec;`,
  ) as (args: Record<string, unknown>) => unknown;

  // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
  const deleteHandler = new Function(
    "args",
    `${seedHydrate}
     var id = String((args && args.id) || "");
     window.__neuralPitchTestHooks.recordings = (window.__neuralPitchTestHooks.recordings || [])
       .filter(function (r) { return r.id !== id; });
     return null;`,
  ) as (args: Record<string, unknown>) => unknown;

  // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
  const renameHandler = new Function(
    "args",
    `${seedHydrate}
     var id = String((args && args.id) || "");
     var label = String((args && args.label) || "");
     window.__neuralPitchTestHooks.recordings = (window.__neuralPitchTestHooks.recordings || [])
       .map(function (r) {
         if (r.id !== id) return r;
         var copy = Object.assign({}, r);
         copy.userLabel = label;
         return copy;
       });
     return null;`,
  ) as (args: Record<string, unknown>) => unknown;

  return {
    list_recordings: listHandler,
    start_recording: startHandler,
    stop_recording: stopHandler,
    delete_recording: deleteHandler,
    rename_recording: renameHandler,
  };
}

/**
 * Push a synthetic `recording-progress` event. Mirrors `pushPitchUpdate`:
 * the page-side `recordingsStore` registers a listener on
 * `__neuralPitchTestHooks.listeners.get("recording-progress")` and tests
 * drive elapsed-time ticks through this helper.
 */
export async function pushRecordingProgress(
  page: Page,
  progress: MockRecordingProgress,
): Promise<void> {
  await page.evaluate((frame) => {
    type WindowWithHooks = Window & {
      __neuralPitchTestHooks?: {
        listeners: Map<string, Array<(payload: unknown) => void>>;
      };
    };
    const w = window as WindowWithHooks;
    const listeners = w.__neuralPitchTestHooks?.listeners.get("recording-progress") ?? [];
    for (const fn of listeners) {
      fn(frame);
    }
  }, progress);
}

/**
 * Phase-2.1 analysis — wire-format mirrors `src/types/analysis.ts` (planned).
 *
 * Field names are camelCase on the TS side; the IPC boundary maps from
 * snake_case Rust per the existing `recording.ts` convention. Specs
 * describe the world in camelCase; the page-side `installAnalysisMock`
 * handlers also return camelCase so React components can consume the
 * payload without re-mapping.
 *
 * `wasCached` is the only field that varies across the cache spec's two
 * branches (cached read vs. forced re-analyze). The mock derives it from
 * the inbound `forceRefresh` flag rather than from a seed entry, so both
 * branches share a single seed shape.
 */
export interface MockAnalysisSummary {
  recordingId: string;
  medianMidi: number; // e.g. 69 for A4
  medianCents: number; // signed, 1 decimal precision in display
  voicedRatio: number; // 0..1
  wasCached: boolean;
  analyzerVersion: string;
}

/** A single voiced/unvoiced frame in the contour timeline. */
export interface MockContourFrame {
  tMs: number;
  centsFromMedian: number;
  voiced: boolean;
}

export interface MockContourResult {
  recordingId: string;
  analyzerVersion: string;
  frames: MockContourFrame[];
  medianMidi: number;
  medianCents: number;
  voicedRatio: number;
}

/**
 * In-flight progress payload — emitted at ~10 Hz over the
 * `analysis-progress` Tauri channel while pYIN/PESTO is running. The
 * page-side `analysisStore` registers a listener on
 * `__neuralPitchTestHooks.listeners.get("analysis-progress")` and
 * pushes the percent into the `<progress role="progressbar">` rendered
 * by AnalysisSummary while the recording id is in `inProgress`.
 */
export interface MockAnalysisProgress {
  recordingId: string;
  percent: number; // 0..100
  status?: "running" | "finalizing" | "failed";
  error?: string;
}

/**
 * Install mock responses for the Phase-2.1 analysis IPC surface.
 *
 * Following the same source-serialisation pattern as `installRecordingsMock`:
 * closures do not survive `Function.prototype.toString()`, so each handler
 * embeds a JSON-encoded literal for the seed maps and self-initialises the
 * shared `__neuralPitchTestHooks.analysis` slot on first call.
 *
 * Phase-2.1 contract:
 *   - `analyze_recording({ recordingId, forceRefresh })`
 *       → MockAnalysisSummary (with `wasCached = !forceRefresh`)
 *   - `get_contour({ recordingId })`
 *       → MockContourResult (frames + median + voiced ratio)
 *
 * @param byRecordingId  Map of `recordingId → MockAnalysisSummary` for the
 *                       seeded summary card.
 * @param contoursByKey  Map of `${recordingId}:${analyzerVersion}` →
 *                       MockContourResult. The composite key matches the
 *                       `contoursByKey` Map in `analysisStore`.
 */
export function installAnalysisMock(
  byRecordingId: Record<string, MockAnalysisSummary>,
  contoursByKey: Record<string, MockContourResult>,
): TauriMockResponses {
  const summaryJson = JSON.stringify(byRecordingId);
  const contourJson = JSON.stringify(contoursByKey);

  const seedHydrate = `
    var w = window;
    var hooks = w.__neuralPitchTestHooks || {};
    if (!hooks.analysis) {
      hooks.analysis = {
        summaries: JSON.parse(${JSON.stringify(summaryJson)}),
        contours: JSON.parse(${JSON.stringify(contourJson)})
      };
    }
    w.__neuralPitchTestHooks = hooks;
  `;

  // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
  const analyzeHandler = new Function(
    "args",
    `${seedHydrate}
     var id = String((args && args.recordingId) || "");
     var force = Boolean(args && args.forceRefresh);
     var seed = window.__neuralPitchTestHooks.analysis.summaries[id];
     if (!seed) {
       throw new Error("unmocked analysis summary for recordingId: " + id);
     }
     var copy = Object.assign({}, seed);
     copy.wasCached = !force;
     return copy;`,
  ) as (args: Record<string, unknown>) => unknown;

  // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
  const contourHandler = new Function(
    "args",
    `${seedHydrate}
     var id = String((args && args.recordingId) || "");
     var contours = window.__neuralPitchTestHooks.analysis.contours;
     // Find first contour entry whose composite key starts with "id:".
     var prefix = id + ":";
     var keys = Object.keys(contours);
     for (var i = 0; i < keys.length; i++) {
       if (keys[i].indexOf(prefix) === 0) {
         return contours[keys[i]];
       }
     }
     throw new Error("unmocked contour for recordingId: " + id);`,
  ) as (args: Record<string, unknown>) => unknown;

  return {
    analyze_recording: analyzeHandler,
    get_contour: contourHandler,
  };
}

/**
 * Push a synthetic `analysis-progress` event. Mirrors `pushRecordingProgress`:
 * the page-side `analysisStore` registers a listener on
 * `__neuralPitchTestHooks.listeners.get("analysis-progress")` and tests
 * drive percent ticks through this helper. When `percent === 100` the
 * spec is expected to resolve the in-flight `analyze_recording()` promise
 * via the IPC mock (which is already synchronous here), so the bar
 * disappears in the same tick.
 */
export async function pushAnalysisProgress(
  page: Page,
  progress: MockAnalysisProgress,
): Promise<void> {
  await page.evaluate((frame) => {
    type WindowWithHooks = Window & {
      __neuralPitchTestHooks?: {
        listeners: Map<string, Array<(payload: unknown) => void>>;
      };
    };
    const w = window as WindowWithHooks;
    const listeners = w.__neuralPitchTestHooks?.listeners.get("analysis-progress") ?? [];
    for (const fn of listeners) {
      fn(frame);
    }
  }, progress);
}

/**
 * Phase-2.3 vocal-range — wire-format mirrors `RangeReport` in
 * `src/types/analysis.ts`. The summary cache extension carries a `range`
 * field next to the existing pYIN/PESTO numerics; readouts read from the
 * same `byRecording` Map entry.
 *
 *   New Grove Dictionary of Music — vocal-range conventions for
 *   `voiceTypeHints` (e.g. ["Alto", "Mezzo-soprano"]).
 */
export interface MockRangeReport {
  comfortableLowMidi: number;
  comfortableHighMidi: number;
  fullLowMidi: number;
  fullHighMidi: number;
  voicedFrameCount: number;
  voiceTypeHints: string[];
}

/**
 * Phase-2.3 vibrato — wire-format mirrors `VibratoReport` in
 * `src/types/analysis.ts`. Per-window dots downstream of the rate bar are
 * driven by the `windows[]` array; the meter (`role="meter"`) reflects
 * `medianRateHz` against the 0–10 Hz scale.
 */
export interface MockVibratoWindow {
  tMs: number;
  rateHz: number;
  extentCents: number;
  confidence: number;
}

export interface MockVibratoReport {
  medianRateHz: number;
  medianExtentCents: number;
  vibratoRatio: number;
  windows: MockVibratoWindow[];
}

/**
 * Convenience wrapper around `installAnalysisMock` that merges a
 * `RangeReport` into each summary entry before delegating. Specs that
 * exercise the vocal-range readout call this so the seeded summary
 * carries `range` directly — no second IPC, no second store entry.
 *
 * Per the Phase-2.3 architecture, the `byRecording`
 * Map already holds `AnalysisSummary`; `RangeReadout` reads
 * `byRecording.get(id)?.range` from the same entry as the existing
 * summary card.
 */
export function installAnalysisMockWithRange(
  byRecordingId: Record<string, MockAnalysisSummary>,
  contoursByKey: Record<string, MockContourResult>,
  rangeByRecordingId: Record<string, MockRangeReport>,
): TauriMockResponses {
  const merged: Record<string, MockAnalysisSummary & { range?: MockRangeReport }> = {};
  for (const [id, summary] of Object.entries(byRecordingId)) {
    const range = rangeByRecordingId[id];
    merged[id] = range !== undefined ? { ...summary, range } : { ...summary };
  }
  return installAnalysisMock(merged as Record<string, MockAnalysisSummary>, contoursByKey);
}

/**
 * Convenience wrapper around `installAnalysisMock` that merges a
 * `VibratoReport` into each summary entry. Mirrors
 * `installAnalysisMockWithRange` for the vibrato readout. Specs that
 * exercise both readouts compose the wrappers by spreading the second
 * call's output over the first — both wrappers funnel through
 * `installAnalysisMock` so the single `analyze_recording` handler
 * resolves with a summary carrying both fields.
 */
export function installAnalysisMockWithVibrato(
  byRecordingId: Record<string, MockAnalysisSummary>,
  contoursByKey: Record<string, MockContourResult>,
  vibratoByRecordingId: Record<string, MockVibratoReport>,
): TauriMockResponses {
  const merged: Record<string, MockAnalysisSummary & { vibrato?: MockVibratoReport }> = {};
  for (const [id, summary] of Object.entries(byRecordingId)) {
    const vibrato = vibratoByRecordingId[id];
    merged[id] = vibrato !== undefined ? { ...summary, vibrato } : { ...summary };
  }
  return installAnalysisMock(merged as Record<string, MockAnalysisSummary>, contoursByKey);
}

/**
 * Phase-2.4 playback fixture.
 *
 * `installPlaybackMock()` returns a `TauriMockResponses` patch that
 * overrides `get_recording_path` to return a sentinel string, plus an
 * adjacent `installPlaybackRoutes(page)` helper that:
 *
 *   1. Synthesises a 1 s, 440 Hz, mono 16-bit PCM WAV in the page itself
 *      (no big base64 blob in the helper source).
 *   2. Overrides `convertFileSrc` so the sentinel resolves to a stable
 *      page-relative URL, bypassing Tauri's `tauri://` bridge.
 *   3. Routes that URL through `page.route()` to deliver the WAV bytes
 *      with `Content-Type: audio/wav`.
 *
 * No real Tauri filesystem, no external network: the bytes never leave
 * the test process.
 */
export const PLAYBACK_FIXTURE_SENTINEL = "e2e:fixture-1khz-1s.wav";
export const PLAYBACK_FIXTURE_URL = "/__e2e/fixture-1khz-1s.wav";

export function installPlaybackMock(): TauriMockResponses {
  // The sentinel is a literal string; the `serialise()` pipeline already
  // routes literal values through the `kind: "value"` branch, so we
  // don't need a `new Function(...)` wrapper here.
  return {
    get_recording_path: PLAYBACK_FIXTURE_SENTINEL,
  };
}

/**
 * Build the 1 s, 440 Hz, mono 16-bit PCM WAV at 8 kHz on the Node side.
 * Small (~16 KB) so a single `Buffer.from` round-trip per spec is cheap.
 */
function buildFixtureWav(): Buffer {
  const sampleRate = 8000;
  const durationS = 1;
  const freqHz = 440;
  const totalSamples = sampleRate * durationS;
  const dataBytes = totalSamples * 2;
  const totalSize = 44 + dataBytes;
  const buf = Buffer.alloc(totalSize);
  buf.write("RIFF", 0, "ascii");
  buf.writeUInt32LE(totalSize - 8, 4);
  buf.write("WAVE", 8, "ascii");
  buf.write("fmt ", 12, "ascii");
  buf.writeUInt32LE(16, 16); // PCM fmt chunk size
  buf.writeUInt16LE(1, 20); // PCM
  buf.writeUInt16LE(1, 22); // mono
  buf.writeUInt32LE(sampleRate, 24);
  buf.writeUInt32LE(sampleRate * 2, 28); // byte rate
  buf.writeUInt16LE(2, 32); // block align
  buf.writeUInt16LE(16, 34); // bits per sample
  buf.write("data", 36, "ascii");
  buf.writeUInt32LE(dataBytes, 40);
  for (let i = 0; i < totalSamples; i++) {
    const v = Math.round(Math.sin((2 * Math.PI * freqHz * i) / sampleRate) * 0.5 * 32767);
    buf.writeInt16LE(v, 44 + i * 2);
  }
  return buf;
}

/**
 * Page-side wiring: route the sentinel URL to the synthetic WAV bytes
 * AND override `convertFileSrc` so the page resolves the sentinel string
 * to that route. Call AFTER `installTauriMock` (the init-script chain is
 * append-only), and BEFORE `page.goto("/")`.
 */
export async function installPlaybackRoutes(page: Page): Promise<void> {
  const wav = buildFixtureWav();
  await page.route(`**${PLAYBACK_FIXTURE_URL}`, async (route) => {
    await route.fulfill({
      status: 200,
      contentType: "audio/wav",
      body: wav,
    });
  });
  await page.addInitScript(
    (cfg: { sentinel: string; url: string }) => {
      type Hooks = {
        handlers?: Map<string, unknown>;
        listeners?: Map<string, Array<(payload: unknown) => void>>;
        invokeCalls?: Array<{ cmd: string; args: Record<string, unknown> }>;
        convertFileSrc?: (path: string) => string;
      };
      type WindowWithHooks = Window & { __neuralPitchTestHooks?: Hooks };
      const w = window as WindowWithHooks;
      const hooks: Hooks = w.__neuralPitchTestHooks ?? {
        handlers: new Map(),
        listeners: new Map(),
        invokeCalls: [],
      };
      // The page-side `convertFileSrc()` import is consulted by
      // PlaybackPanel via a thin shim that prefers this hook when set,
      // mirroring how `usePitchStream` consults `__neuralPitchTestHooks
      // .listeners` instead of the real Tauri channel API.
      hooks.convertFileSrc = (path: string): string => {
        if (path === cfg.sentinel) return cfg.url;
        return path;
      };
      w.__neuralPitchTestHooks = hooks;
    },
    { sentinel: PLAYBACK_FIXTURE_SENTINEL, url: PLAYBACK_FIXTURE_URL },
  );
}

/**
 * Phase-3 transcription — wire-format mirrors `src/types/transcription.ts`
 * (planned). Field names are camelCase on the TS side; the Rust IPC boundary
 * maps from snake_case per the existing `analysis.ts` convention.
 *
 * `wasCached` is the only field that varies across the cache branches
 * (cached read vs. forced re-transcribe). The mock derives it from the
 * inbound `forceRefresh` flag, mirroring `installAnalysisMock`.
 */
export interface MockTranscribeSummary {
  recordingId: string;
  noteCount: number;
  durationMs: number;
  wasCached: boolean;
  transcriberVersion: string;
}

/** A single point in the per-note `pitch_bend_curve` polyline. */
export interface MockBendPoint {
  tMs: number;
  cents: number;
}

/** A single note in the polyphonic transcription. */
export interface MockNote {
  midi: number; // 21..108
  startMs: number;
  durationMs: number;
  velocity: number; // 0..127
  pitchBendCurve: MockBendPoint[];
}

export interface MockPolyResult {
  recordingId: string;
  transcriberVersion: string;
  durationMs: number;
  notes: MockNote[];
}

/**
 * In-flight transcription progress payload — emitted at ~10 Hz over the
 * `transcribe-progress` Tauri channel while Basic Pitch / ONNX inference
 * is running. The page-side `transcriptionStore` registers a listener on
 * `__neuralPitchTestHooks.listeners.get("transcribe-progress")` and pushes
 * the percent into the `<progress role="progressbar">` rendered by
 * TranscribePanel while the recording id is in `inProgress`.
 */
export interface MockTranscribeProgress {
  recordingId: string;
  percent: number; // 0..100
  status?: "running" | "finalizing" | "failed";
  error?: string;
}

/**
 * Build a small synthetic 3-note `PolyResult` over 1.2 s — E4, G4, B4.
 * Each note carries a 5-point linear `pitch_bend_curve`. Specs reuse this
 * factory verbatim so the canvas hit-tests are deterministic and the axe
 * scan has a stable label fragment to match against.
 */
export function buildSyntheticPolyResult(recordingId: string): MockPolyResult {
  const linearBend = (startMs: number, durationMs: number): MockBendPoint[] => {
    const points: MockBendPoint[] = [];
    for (let i = 0; i < 5; i += 1) {
      const fraction = i / 4;
      points.push({
        tMs: startMs + fraction * durationMs,
        cents: -10 + 20 * fraction,
      });
    }
    return points;
  };
  return {
    recordingId,
    transcriberVersion: "basicpitch-0.1.0",
    durationMs: 1200,
    notes: [
      {
        midi: 64, // E4
        startMs: 0,
        durationMs: 400,
        velocity: 96,
        pitchBendCurve: linearBend(0, 400),
      },
      {
        midi: 67, // G4
        startMs: 400,
        durationMs: 400,
        velocity: 102,
        pitchBendCurve: linearBend(400, 400),
      },
      {
        midi: 71, // B4
        startMs: 800,
        durationMs: 400,
        velocity: 108,
        pitchBendCurve: linearBend(800, 400),
      },
    ],
  };
}

/**
 * Install mock responses for the Phase-3 import IPC surface.
 *
 * Phase-3 contract:
 *   - `import_audio_file({ sourcePath })` → MockRecording (synthetic row
 *     pushed onto the shared `__neuralPitchTestHooks.recordings` slot).
 *
 * The handler self-initialises the slot on first call using the embedded
 * seed JSON — same closure-survival pattern as `installRecordingsMock`.
 */
export function installImportMock(): TauriMockResponses {
  // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
  const importHandler = new Function(
    "args",
    `var w = window;
     var hooks = w.__neuralPitchTestHooks || {};
     if (!hooks.recordings) { hooks.recordings = []; }
     w.__neuralPitchTestHooks = hooks;
     var src = String((args && args.sourcePath) || "/tmp/imported.wav");
     var slash = src.lastIndexOf("/");
     var base = slash >= 0 ? src.substring(slash + 1) : src;
     var id = "rec-import-" + Date.now() + "-" + Math.floor(Math.random() * 1e6).toString(16);
     var rec = {
       id: id,
       filename: base,
       createdAt: Date.now(),
       durationMs: 1230,
       sampleRateHz: 48000,
       channels: 1,
       bitDepth: 24,
       a4Hz: 440,
       instrumentProfile: "Voice"
     };
     hooks.recordings = [rec].concat(hooks.recordings || []);
     w.__neuralPitchTestHooks = hooks;
     return rec;`,
  ) as (args: Record<string, unknown>) => unknown;

  return {
    import_audio_file: importHandler,
  };
}

/**
 * Page-side stub for the Tauri `plugin-dialog` `open()` call. The Phase-3
 * `ImportButton` issues `await open({ multiple: false, filters: [...] })`
 * and routes the resulting path string through `import_audio_file`. We
 * stash a fixed sentinel on `__neuralPitchTestHooks.dialogOpenResult` and
 * the page-side dialog shim consults that hook before falling back to the
 * real plugin — exactly how `convertFileSrc` is overridden in
 * `installPlaybackRoutes`.
 *
 * Pass `null` to simulate the user dismissing the dialog.
 */
export async function installDialogMock(
  page: Page,
  result: string | null = "/tmp/imported-fixture.wav",
): Promise<void> {
  await page.addInitScript((seed: string | null) => {
    type Hooks = {
      handlers?: Map<string, unknown>;
      listeners?: Map<string, Array<(payload: unknown) => void>>;
      invokeCalls?: Array<{ cmd: string; args: Record<string, unknown> }>;
      dialogOpenResult?: string | null;
    };
    type WindowWithHooks = Window & { __neuralPitchTestHooks?: Hooks };
    const w = window as WindowWithHooks;
    const hooks: Hooks = w.__neuralPitchTestHooks ?? {
      handlers: new Map(),
      listeners: new Map(),
      invokeCalls: [],
    };
    hooks.dialogOpenResult = seed;
    w.__neuralPitchTestHooks = hooks;
  }, result);
}

/**
 * Install mock responses for the Phase-3 transcription IPC surface.
 *
 * Phase-3 contract:
 *   - `transcribe_recording({ recordingId, forceRefresh })`
 *       → MockTranscribeSummary (with `wasCached = !forceRefresh`)
 *   - `get_poly_result({ recordingId })`
 *       → MockPolyResult (3-note synthetic seed by default)
 *   - `export_midi({ recordingId, destPath })`
 *       → null (records the call for assertion)
 *
 * @param byRecordingId  Map of `recordingId → MockTranscribeSummary` for
 *                       the seeded summary.
 * @param polyByKey      Map of `${recordingId}:${transcriberVersion}` →
 *                       MockPolyResult. The composite key matches the
 *                       `polyResultsByKey` Map in `transcriptionStore`.
 */
export function installTranscribeMock(
  byRecordingId: Record<string, MockTranscribeSummary>,
  polyByKey: Record<string, MockPolyResult>,
): TauriMockResponses {
  const summaryJson = JSON.stringify(byRecordingId);
  const polyJson = JSON.stringify(polyByKey);

  const seedHydrate = `
    var w = window;
    var hooks = w.__neuralPitchTestHooks || {};
    if (!hooks.transcription) {
      hooks.transcription = {
        summaries: JSON.parse(${JSON.stringify(summaryJson)}),
        polyResults: JSON.parse(${JSON.stringify(polyJson)}),
        exports: []
      };
    }
    w.__neuralPitchTestHooks = hooks;
  `;

  // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
  const transcribeHandler = new Function(
    "args",
    `${seedHydrate}
     var id = String((args && args.recordingId) || "");
     var force = Boolean(args && args.forceRefresh);
     var seed = window.__neuralPitchTestHooks.transcription.summaries[id];
     if (!seed) {
       throw new Error("unmocked transcribe summary for recordingId: " + id);
     }
     var copy = Object.assign({}, seed);
     copy.wasCached = !force;
     return copy;`,
  ) as (args: Record<string, unknown>) => unknown;

  // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
  const polyHandler = new Function(
    "args",
    `${seedHydrate}
     var id = String((args && args.recordingId) || "");
     var polyResults = window.__neuralPitchTestHooks.transcription.polyResults;
     var prefix = id + ":";
     var keys = Object.keys(polyResults);
     for (var i = 0; i < keys.length; i++) {
       if (keys[i].indexOf(prefix) === 0) {
         return polyResults[keys[i]];
       }
     }
     throw new Error("unmocked poly result for recordingId: " + id);`,
  ) as (args: Record<string, unknown>) => unknown;

  // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
  const exportHandler = new Function(
    "args",
    `${seedHydrate}
     var id = String((args && args.recordingId) || "");
     var dest = String((args && args.destPath) || "/tmp/export.mid");
     window.__neuralPitchTestHooks.transcription.exports.push({
       recordingId: id,
       destPath: dest
     });
     return null;`,
  ) as (args: Record<string, unknown>) => unknown;

  return {
    transcribe_recording: transcribeHandler,
    get_poly_result: polyHandler,
    export_midi: exportHandler,
  };
}

/**
 * Push a synthetic `transcribe-progress` event. Mirrors
 * `pushAnalysisProgress`: the page-side `transcriptionStore` registers a
 * listener on `__neuralPitchTestHooks.listeners.get("transcribe-progress")`
 * and tests drive percent ticks through this helper. When `percent === 100`
 * the spec is expected to resolve the in-flight `transcribe_recording()`
 * promise via the IPC mock (which is already synchronous here), so the bar
 * disappears in the same tick.
 */
export async function pushTranscribeProgress(
  page: Page,
  progress: MockTranscribeProgress,
): Promise<void> {
  await page.evaluate((frame) => {
    type WindowWithHooks = Window & {
      __neuralPitchTestHooks?: {
        listeners: Map<string, Array<(payload: unknown) => void>>;
      };
    };
    const w = window as WindowWithHooks;
    const listeners = w.__neuralPitchTestHooks?.listeners.get("transcribe-progress") ?? [];
    for (const fn of listeners) {
      fn(frame);
    }
  }, progress);
}

/**
 * Phase-4 ear-training drills — wire-format mirrors `src/types/training.ts`
 * (planned). Field names are camelCase on the TS side; the Rust IPC
 * boundary maps from snake_case per the existing `transcription.ts`
 * convention.
 *
 * The drill subsystem is opt-in: a "Practice" header button flips
 * `tunerStore.view` to `"training"` and the `Training` screen renders the
 * drill cards. Each card mounts a single-screen drill component (Interval,
 * Chord, Scale, Sight-singing, Tuning practice) that owns its own prompt
 * loop and final-score toast.
 */

/** A single completed drill attempt — what feeds the "last attempt" stats. */
export interface MockDrillAttempt {
  id: string;
  drillId: "intervals" | "chords" | "scales" | "sight-singing" | "tuning";
  startedAt: number; // ms epoch
  completedAt: number; // ms epoch
  totalPrompts: number;
  correctCount: number;
  accuracy: number; // 0..1
}

/** A single note in the sight-singing target melody. */
export interface MockMelodyNote {
  midi: number; // 21..108
  startMs: number;
  durationMs: number;
}

export interface MockMelody {
  id: string;
  tonicMidi: number; // for movable-do solfege rendering
  notes: MockMelodyNote[];
}

/**
 * Per-frame match update emitted by `start_drill_match` over a
 * `Channel<MatchUpdate>`. The page-side store writes incoming updates to
 * `liveMatch` and the KaraokeRibbon repaints. Mirrors the Rust enum
 * variants with snake_case discriminants for parity with PitchUpdate.
 */
export interface MockMatchUpdate {
  t_ms: number;
  target_midi: number;
  current_midi: number;
  cents_offset: number;
  in_tune: boolean;
  bar_index: number;
  ended: boolean; // true on the final frame
}

/**
 * Install mock responses for the Phase-4 ear-training IPC surface.
 *
 * Phase-4 contract:
 *   - `start_drill_match({ melody })` → null (registers a Channel listener
 *     on the page-side store; tests drive frames via `pushMatchUpdate`).
 *   - `stop_drill_match()` → null (no-op; the receiver tear-down is
 *     idempotent on the Rust side).
 *
 * The handler self-initialises `__neuralPitchTestHooks.training` from the
 * embedded seed JSON on first call — same closure-survival pattern as
 * `installRecordingsMock` and `installTranscribeMock`.
 *
 * @param seedHistory  Pre-seeded drill attempt history. The Training
 *                     landing reads the latest entry per drillId for the
 *                     "last attempt" copy on each card.
 * @param seedMelody   Pre-seeded sight-singing melody — KaraokeRibbon
 *                     paints these as target bars.
 */
export function installTrainingMock(
  seedHistory: MockDrillAttempt[],
  seedMelody: MockMelody,
): TauriMockResponses {
  const historyJson = JSON.stringify(seedHistory);
  const melodyJson = JSON.stringify(seedMelody);

  const seedHydrate = `
    var w = window;
    var hooks = w.__neuralPitchTestHooks || {};
    if (!hooks.training) {
      hooks.training = {
        history: JSON.parse(${JSON.stringify(historyJson)}),
        melody: JSON.parse(${JSON.stringify(melodyJson)}),
        matchQueue: [],
        audioPlayCount: 0
      };
    }
    w.__neuralPitchTestHooks = hooks;
  `;

  // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
  const startHandler = new Function(
    "args",
    `${seedHydrate}
     window.__neuralPitchTestHooks.training.activeMelody =
       (args && args.melody) || window.__neuralPitchTestHooks.training.melody;
     return null;`,
  ) as (args: Record<string, unknown>) => unknown;

  // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
  const stopHandler = new Function(
    "args",
    `${seedHydrate}
     window.__neuralPitchTestHooks.training.activeMelody = undefined;
     return null;`,
  ) as (args: Record<string, unknown>) => unknown;

  // List the seeded history. The Training landing invokes this on mount to
  // hydrate the per-card "last attempt" copy. Mirrors the production
  // `list_drill_history` IPC; the seed shape already matches the TS-side
  // `DrillAttempt` so the response passes through unchanged.
  // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
  const listHandler = new Function(
    "args",
    `${seedHydrate}
     return (window.__neuralPitchTestHooks.training.history || []).slice();`,
  ) as (args: Record<string, unknown>) => unknown;

  return {
    start_drill_match: startHandler,
    stop_drill_match: stopHandler,
    list_drill_history: listHandler,
  };
}

/**
 * Push a synthetic `match-update` event through the test bridge. Mirrors
 * `pushPitchUpdate`: the page-side training store registers a listener on
 * `__neuralPitchTestHooks.listeners.get("match-update")` and the helper
 * walks that list to deliver the payload.
 *
 * MUST be a no-op when the listener list is empty — the React effect can
 * tear down before the test pushes the final frame, and the Rust Channel
 * tolerates the receiver closing early. Mirroring that contract here keeps
 * the spec resilient to legitimate unmount races.
 */
export async function pushMatchUpdate(page: Page, update: MockMatchUpdate): Promise<void> {
  await page.evaluate((frame) => {
    type WindowWithHooks = Window & {
      __neuralPitchTestHooks?: {
        listeners: Map<string, Array<(payload: unknown) => void>>;
      };
    };
    const w = window as WindowWithHooks;
    const listeners = w.__neuralPitchTestHooks?.listeners.get("match-update") ?? [];
    if (listeners.length === 0) return; // receiver-closed-early no-op
    for (const fn of listeners) {
      fn(frame);
    }
  }, update);
}

/**
 * Build a deterministic 8-note ascending C-major melody for the
 * sight-singing drill. Each note is 250 ms long; total melody is 2 s.
 * Returned as a `MockMelody` with `tonicMidi = 60` (C4) so movable-do
 * resolves to "Do, Re, Mi, Fa, Sol, La, Ti, Do".
 */
export function buildSyntheticMelody(): MockMelody {
  const tonicMidi = 60; // C4
  const stepsFromTonic = [0, 2, 4, 5, 7, 9, 11, 12];
  const notes: MockMelodyNote[] = stepsFromTonic.map((step, i) => ({
    midi: tonicMidi + step,
    startMs: i * 250,
    durationMs: 250,
  }));
  return { id: "melody-c-major-octave", tonicMidi, notes };
}

/**
 * Phase-5 stems — wire-format mirrors `src/types/stems.ts`. Field names
 * are camelCase on the TS side; the Rust IPC boundary maps from
 * snake_case per the existing `transcription.ts` convention.
 */

/** The four standard Demucs stems. */
export type MockStemKind = "vocals" | "drums" | "bass" | "other";

/** Per-frame separation progress payload — emitted at ~10–20 Hz over the
 *  `separate-progress` Tauri channel while HTDemucs is running. */
export interface MockSeparateProgress {
  recordingId: string;
  stage: MockStemKind | "finalizing";
  percent: number; // 0..100
}

/** Static metadata about the bundled HTDemucs model. The mock surfaces a
 *  `cached: true` flag so the panel skips the download arc by default. */
export interface MockStemModelInfo {
  downloadUrl: string;
  sha256: string;
  sizeBytes: number;
}

/**
 * Install mock responses for the Phase-5 stem-separation IPC surface.
 *
 * Phase-5 contract:
 *   - `download_stem_model()`                    -> `{ cached: true }`
 *   - `get_stem_model_info()`                    -> `MockStemModelInfo`
 *   - `separate_stems({ recordingId })`          -> parked promise; resolves
 *     only when `pushStemsComplete(page, ...)` fires (or rejects when
 *     `cancel_stem_separation` is invoked, or when `pushStemsError` fires).
 *   - `cancel_stem_separation({ recordingId })`  -> null (best-effort; the
 *     in-page panel's own pending-promise registry drives the reject).
 *   - `get_stem_path({ recordingId, stemKind })` -> sentinel path string.
 *   - `read_stem_audio()`                        -> 0-length Uint8Array
 *     stand-in (not consumed by the panel — PlaybackPanel resolves the
 *     path via `convertFileSrc`, not this IPC).
 *   - `export_stem({ recordingId, stemKind, destPath })` -> null.
 *
 * The handler self-initialises `__neuralPitchTestHooks.stems` from the
 * embedded seed JSON on first call — same closure-survival pattern as
 * `installRecordingsMock` and `installTranscribeMock`.
 *
 * Note: PlaybackPanel resolves stem audio via the same `convertFileSrc`
 * test-hook path used by the mix; the helper returns a sentinel that
 * routes through the `installPlaybackRoutes` resolver if the test wires
 * one up. Specs that don't exercise audio playback can let wavesurfer
 * 404 — the spec assertions only target the panel chrome and progress
 * markup.
 */
export function installStemsMock(): TauriMockResponses {
  // Hydrator. Each handler embeds this so cold-call into any handler
  // initialises the shared slot. Closures don't survive the
  // `Function.prototype.toString()` round-trip, so we cannot capture an
  // outer Map — every handler has to re-derive state from the hooks slot.
  const seedHydrate = `
    var w = window;
    var hooks = w.__neuralPitchTestHooks || {};
    if (!hooks.stems) {
      hooks.stems = {
        // Pending separations keyed by recordingId. Each slot holds a
        // pair of resolver functions the channel-side helpers fire.
        pendingResolvers: {},
        pendingRejecters: {},
        // Fixed sentinel paths so each stem maps to a stable URL the
        // PlaybackPanel mount path can pass through convertFileSrc.
        stemPaths: {},
        // Last-export call for assertion if a spec needs it.
        exports: []
      };
    }
    w.__neuralPitchTestHooks = hooks;
  `;

  // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
  const downloadHandler = new Function(
    "args",
    `${seedHydrate}
     return { cached: true };`,
  ) as (args: Record<string, unknown>) => unknown;

  // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
  const modelInfoHandler = new Function(
    "args",
    `${seedHydrate}
     return {
       downloadUrl: "https://example.invalid/htdemucs.onnx",
       sha256: "0000000000000000000000000000000000000000000000000000000000000000",
       sizeBytes: 80 * 1024 * 1024
     };`,
  ) as (args: Record<string, unknown>) => unknown;

  // The separate handler returns a Promise that the page-side helpers
  // resolve / reject. We stash the resolvers on the hooks slot keyed by
  // recordingId so `pushStemsComplete` and `pushStemsError` can find
  // them. `cancel_stem_separation` rejects with an Error("Cancelled") which
  // the store's catch path uses to flip back to `idle`.
  // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
  const separateHandler = new Function(
    "args",
    `${seedHydrate}
     var id = String((args && args.recordingId) || "");
     return new Promise(function (resolve, reject) {
       window.__neuralPitchTestHooks.stems.pendingResolvers[id] = resolve;
       window.__neuralPitchTestHooks.stems.pendingRejecters[id] = reject;
     });`,
  ) as (args: Record<string, unknown>) => unknown;

  // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
  const cancelHandler = new Function(
    "args",
    `${seedHydrate}
     var id = String((args && args.recordingId) || "");
     var rej = window.__neuralPitchTestHooks.stems.pendingRejecters[id];
     if (typeof rej === "function") {
       delete window.__neuralPitchTestHooks.stems.pendingResolvers[id];
       delete window.__neuralPitchTestHooks.stems.pendingRejecters[id];
       try { rej(new Error("Cancelled")); } catch (e) { /* swallow */ }
     }
     return null;`,
  ) as (args: Record<string, unknown>) => unknown;

  // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
  const getPathHandler = new Function(
    "args",
    `${seedHydrate}
     var id = String((args && args.recordingId) || "");
     var kind = String((args && args.stemKind) || "vocals");
     return "/tmp/stems/" + id + "/" + kind + ".flac";`,
  ) as (args: Record<string, unknown>) => unknown;

  // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
  const readStemHandler = new Function(
    "args",
    `${seedHydrate}
     return new Uint8Array(0);`,
  ) as (args: Record<string, unknown>) => unknown;

  // eslint-disable-next-line @typescript-eslint/no-implied-eval, no-new-func
  const exportHandler = new Function(
    "args",
    `${seedHydrate}
     window.__neuralPitchTestHooks.stems.exports.push({
       recordingId: String((args && args.recordingId) || ""),
       stemKind: String((args && args.stemKind) || ""),
       destPath: String((args && args.destPath) || "")
     });
     return null;`,
  ) as (args: Record<string, unknown>) => unknown;

  return {
    download_stem_model: downloadHandler,
    get_stem_model_info: modelInfoHandler,
    separate_stems: separateHandler,
    cancel_stem_separation: cancelHandler,
    get_stem_path: getPathHandler,
    read_stem_audio: readStemHandler,
    export_stem: exportHandler,
  };
}

/**
 * Push a synthetic `separate-progress` event through the test bridge.
 * Mirrors `pushTranscribeProgress`: the page-side `useStemProgressSubscription`
 * hook registers a listener on `__neuralPitchTestHooks.listeners.get(
 * "separate-progress")` and tests drive frames through this helper.
 *
 * MUST be a no-op when the listener list is empty — receiver-closed-early
 * tolerance for unmount races (e.g. cancel-then-late-frame).
 */
export async function pushStemsProgress(page: Page, progress: MockSeparateProgress): Promise<void> {
  await page.evaluate((frame) => {
    type WindowWithHooks = Window & {
      __neuralPitchTestHooks?: {
        listeners: Map<string, Array<(payload: unknown) => void>>;
      };
    };
    const w = window as WindowWithHooks;
    const listeners = w.__neuralPitchTestHooks?.listeners.get("separate-progress") ?? [];
    if (listeners.length === 0) return; // receiver-closed-early no-op
    for (const fn of listeners) {
      fn(frame);
    }
  }, progress);
}

/**
 * Resolve the parked `separate_stems` promise for a recording id with
 * the four FLAC paths. The store flips to `complete` and the panel
 * mounts the four StemCards.
 */
export async function pushStemsComplete(
  page: Page,
  payload: {
    recordingId: string;
    stemPaths?: Partial<Record<MockStemKind, string>>;
  },
): Promise<void> {
  await page.evaluate((p) => {
    type Hooks = {
      stems?: {
        pendingResolvers: Record<string, (value: unknown) => void>;
        pendingRejecters: Record<string, (err: unknown) => void>;
      };
    };
    type WindowWithHooks = Window & { __neuralPitchTestHooks?: Hooks };
    const w = window as WindowWithHooks;
    const stems = w.__neuralPitchTestHooks?.stems;
    if (stems === undefined) return;
    const id = p.recordingId;
    const resolver = stems.pendingResolvers[id];
    if (typeof resolver !== "function") return;
    delete stems.pendingResolvers[id];
    delete stems.pendingRejecters[id];
    const paths = {
      vocals: p.stemPaths?.vocals ?? `/tmp/stems/${id}/vocals.flac`,
      drums: p.stemPaths?.drums ?? `/tmp/stems/${id}/drums.flac`,
      bass: p.stemPaths?.bass ?? `/tmp/stems/${id}/bass.flac`,
      other: p.stemPaths?.other ?? `/tmp/stems/${id}/other.flac`,
    };
    resolver({ stemPaths: paths });
  }, payload);
}

/**
 * Reject the parked `separate_stems` promise with an error. Drives the
 * store into the `error` branch.
 */
export async function pushStemsError(
  page: Page,
  payload: { recordingId: string; message: string },
): Promise<void> {
  await page.evaluate((p) => {
    type Hooks = {
      stems?: {
        pendingResolvers: Record<string, (value: unknown) => void>;
        pendingRejecters: Record<string, (err: unknown) => void>;
      };
    };
    type WindowWithHooks = Window & { __neuralPitchTestHooks?: Hooks };
    const w = window as WindowWithHooks;
    const stems = w.__neuralPitchTestHooks?.stems;
    if (stems === undefined) return;
    const id = p.recordingId;
    const rejecter = stems.pendingRejecters[id];
    if (typeof rejecter !== "function") return;
    delete stems.pendingResolvers[id];
    delete stems.pendingRejecters[id];
    rejecter(new Error(p.message));
  }, payload);
}

/** Recorded invoke calls for assertion. */
export async function getInvokeCalls(
  page: Page,
  cmd?: string,
): Promise<Array<{ cmd: string; args: Record<string, unknown> }>> {
  return await page.evaluate((filterCmd) => {
    type WindowWithHooks = Window & {
      __neuralPitchTestHooks?: {
        invokeCalls: Array<{ cmd: string; args: Record<string, unknown> }>;
      };
    };
    const w = window as WindowWithHooks;
    const all = w.__neuralPitchTestHooks?.invokeCalls ?? [];
    if (typeof filterCmd === "string") return all.filter((c) => c.cmd === filterCmd);
    return all;
  }, cmd ?? null);
}
