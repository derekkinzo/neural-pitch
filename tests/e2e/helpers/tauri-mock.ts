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
    type WindowWithHooks = Window & {
      __E2E__?: boolean;
      __TAURI_INTERNALS__?: Internals;
      __neuralPitchTestHooks?: {
        handlers: Map<string, Handler | unknown>;
        listeners: Map<string, Array<(payload: unknown) => void>>;
        invokeCalls: Array<{ cmd: string; args: Record<string, unknown> }>;
      };
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

    w.__neuralPitchTestHooks = {
      handlers,
      listeners: new Map(),
      invokeCalls: [],
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
      w.__neuralPitchTestHooks?.invokeCalls.push({ cmd, args: a });
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
