// Stems store — slow-path Zustand state for the
// HTDemucs StemSeparationPanel.
//
// The hot path (per-frame `percent` published at ~10–20 Hz from Rust)
// does NOT pass through Zustand. The panel reads percent through a
// `useRef<number>` updated by the channel callback, then writes the
// progress bar's `value` + aria attrs from a rAF loop. Zustand only
// carries the discrete state machine (`idle` → `downloading-model` →
// `separating` → `complete` | `error`) plus the latest `stage` so the
// progress label can re-render when the stem changes.
//
// IPC surface (mirrors INTEGRATION-SPEC):
//   - `download_stem_model()`                   -> { cached: boolean }
//   - `get_stem_model_info()`                   -> StemModelInfo
//   - `separate_stems({ recordingId })`         -> { stemPaths: Record<StemKind, string> }
//   - `cancel_stem_separation({ recordingId })` -> null
//   - `separate-progress` event channel emits SeparateProgress at ~10 Hz
//
// Receiver-closed-early contract (global rule): `_onProgress` no-ops when
// `perRecording[id]?.status !== "separating"`. The IPC handler also
// tolerates the parked promise rejecting before any frames land — `cancel`
// drives that path.

import { create } from "zustand";
import { invoke } from "@tauri-apps/api/core";
import type { RecordingId } from "@/types/recording";
import type { SeparateProgress, SeparateStage, StemKind, StemSeparationState } from "@/types/stems";

interface WireProgress {
  recordingId?: string;
  recording_id?: string;
  stage?: SeparateStage;
  percent?: number;
}

interface WireSeparateResponse {
  stemPaths?: Partial<Record<StemKind, string>>;
  stem_paths?: Partial<Record<StemKind, string>>;
}

/** Normalise a wire-format `separate-progress` payload (snake_case from
 *  the Rust shell, camelCase from the E2E mock). */
export function __normaliseStemsProgress(raw: WireProgress): SeparateProgress {
  return {
    recordingId: raw.recordingId ?? raw.recording_id ?? "",
    stage: raw.stage ?? "vocals",
    percent: typeof raw.percent === "number" ? raw.percent : 0,
  };
}

/** Process-wide ref bus for the per-frame `percent`. The progress bar
 *  reads this value inside a rAF loop and writes it directly to the
 *  underlying `<progress>` element + ARIA attributes — bypassing React
 *  re-renders entirely. Mirrors `lib/playback-head` in spirit. */
const stemPercentByRecording = new Map<RecordingId, number>();

export function readStemPercent(id: RecordingId): number {
  return stemPercentByRecording.get(id) ?? 0;
}

export function writeStemPercent(id: RecordingId, percent: number): void {
  stemPercentByRecording.set(id, Math.max(0, Math.min(100, percent)));
}

export function resetStemPercent(id: RecordingId): void {
  stemPercentByRecording.delete(id);
}

interface ActiveProgress {
  readonly recordingId: RecordingId;
  readonly stage: SeparateStage;
}

export interface StemsState {
  perRecording: Map<RecordingId, StemSeparationState>;
  /** Carries the latest (recordingId, stage) so the progress label can
   *  re-render at stage transitions only — the per-frame `percent` flows
   *  through the ref bus, not React state. */
  activeProgress: ActiveProgress | null;
  /** Polite live-region copy keyed by `recordingId` so two concurrent
   *  separations cannot interleave their stage announcements through a
   *  single shared slot. The panel reads
   *  `liveStatusByRecording.get(recordingId) ?? liveStatus` so existing
   *  selectors still work. */
  liveStatus: string | null;
  liveStatusByRecording: Map<RecordingId, string>;
}

export interface StemsActions {
  separate: (id: RecordingId) => Promise<void>;
  cancel: (id: RecordingId) => void;
  /** Channel-callback action — writes the per-frame `percent` to the ref
   *  bus and only updates Zustand state on stage transitions. */
  applyProgress: (p: SeparateProgress) => void;
  /** Test-only helper to reset state between specs. */
  __resetForTest: () => void;
}

export type StemsStore = StemsState & StemsActions;

/** Per-id pending separation resolver. The IPC promise resolves to the
 *  four FLAC paths; the store action awaits this promise before flipping
 *  to `complete`. */
const pendingSeparations = new Map<RecordingId, { reject: (err: unknown) => void }>();

function shallowCopyMap<K, V>(m: Map<K, V>): Map<K, V> {
  return new Map(m);
}

function setRecordingState(
  prev: Map<RecordingId, StemSeparationState>,
  id: RecordingId,
  next: StemSeparationState,
): Map<RecordingId, StemSeparationState> {
  const copy = shallowCopyMap(prev);
  copy.set(id, next);
  return copy;
}

/** Display label for a stage in panel status copy + aria-valuetext. */
export function stageDisplayLabel(stage: SeparateStage): string {
  if (stage === "finalizing") return "Finalizing";
  // "Separating vocals" / "Separating drums" / etc.
  return `Separating ${stage}`;
}

/** Compose aria-valuetext from a stage + percent. Kept here so the panel
 *  and tests share one source of truth. */
export function stemsAriaValueText(stage: SeparateStage, percent: number): string {
  const rounded = Math.max(0, Math.min(100, Math.round(percent)));
  return `${stageDisplayLabel(stage)}, ${rounded} percent complete`;
}

function setLiveStatus(
  prev: Map<RecordingId, string>,
  id: RecordingId,
  next: string | null,
): Map<RecordingId, string> {
  const copy = new Map(prev);
  if (next === null) copy.delete(id);
  else copy.set(id, next);
  return copy;
}

export const useStemsStore = create<StemsStore>((set, get) => ({
  perRecording: new Map(),
  activeProgress: null,
  liveStatus: null,
  liveStatusByRecording: new Map(),

  separate: async (id) => {
    // Idempotent guard — re-entry while a separation is already running
    // for this recording is a no-op so a double-click on the button does
    // not double-dispatch the IPC.
    const cur = get().perRecording.get(id);
    if (cur?.status === "separating" || cur?.status === "downloading-model") return;

    // Reset the ref bus before any frames land so a stale percent from a
    // previous separation does not bleed through.
    resetStemPercent(id);

    set((s) => ({
      perRecording: setRecordingState(s.perRecording, id, { status: "downloading-model" }),
      liveStatus: "Preparing separation model",
      liveStatusByRecording: setLiveStatus(
        s.liveStatusByRecording,
        id,
        "Preparing separation model",
      ),
    }));

    try {
      // Step 1 — model gate. The Rust shell verifies the cache (and
      // downloads on first use). The mock resolves immediately.
      await invoke<{ cached?: boolean }>("download_stem_model");

      // Re-check the live state — a `cancel()` between download_stem_model
      // resolving and this point should abort the dispatch.
      if (get().perRecording.get(id)?.status !== "downloading-model") return;

      set((s) => ({
        perRecording: setRecordingState(s.perRecording, id, { status: "separating" }),
        activeProgress: { recordingId: id, stage: "vocals" },
        liveStatus: stageDisplayLabel("vocals"),
        liveStatusByRecording: setLiveStatus(
          s.liveStatusByRecording,
          id,
          stageDisplayLabel("vocals"),
        ),
      }));

      // Park the cancellation handle BEFORE dispatching the IPC so a
      // racing `cancel()` can find it. The reject closure rejects the
      // IPC promise via the mock's `cancel_stem_separation` plumbing —
      // the production shell drops the work via the same code path.
      const cancellable = new Promise<void>((_resolve, reject) => {
        pendingSeparations.set(id, { reject });
      });

      // Race: the IPC promise vs. an explicit cancel. Whichever settles
      // first wins. The mock returns the IPC promise once
      // `pushStemsComplete` (or `cancel_stem_separation`) fires.
      const ipcPromise = invoke<WireSeparateResponse>("separate_stems", { recordingId: id });
      const raw = (await Promise.race([ipcPromise, cancellable])) as
        | WireSeparateResponse
        | undefined;

      // If the cancel path won the race the cancellable Promise rejected;
      // the catch branch below drives the cleanup. The IPC win path lands
      // here with the four FLAC paths.
      pendingSeparations.delete(id);

      const stemPaths = raw?.stemPaths ?? raw?.stem_paths ?? {};
      const resolvedPaths: Record<StemKind, string> = {
        vocals: stemPaths.vocals ?? "",
        drums: stemPaths.drums ?? "",
        bass: stemPaths.bass ?? "",
        other: stemPaths.other ?? "",
      };

      set((s) => ({
        perRecording: setRecordingState(s.perRecording, id, {
          status: "complete",
          stemPaths: resolvedPaths,
        }),
        activeProgress: null,
        liveStatus: "Stem separation complete",
        liveStatusByRecording: setLiveStatus(
          s.liveStatusByRecording,
          id,
          "Stem separation complete",
        ),
      }));
      // The percent ref is no longer authoritative — clear it so a
      // subsequent re-run starts at 0.
      resetStemPercent(id);
    } catch (err: unknown) {
      pendingSeparations.delete(id);
      const msg = err instanceof Error ? err.message : String(err);
      const wasCancelled = /cancel/i.test(msg);
      const nextStatusCopy = wasCancelled
        ? "Stem separation cancelled"
        : `Stem separation failed: ${msg}`;
      set((s) => ({
        perRecording: setRecordingState(
          s.perRecording,
          id,
          wasCancelled ? { status: "idle" } : { status: "error", error: msg },
        ),
        activeProgress: null,
        liveStatus: nextStatusCopy,
        liveStatusByRecording: setLiveStatus(s.liveStatusByRecording, id, nextStatusCopy),
      }));
      resetStemPercent(id);
    }
  },

  cancel: (id) => {
    const cur = get().perRecording.get(id);
    if (cur === undefined) return;
    if (cur.status !== "separating" && cur.status !== "downloading-model") return;
    // Best-effort: notify the Rust shell the user gave up. The mock
    // rejects the parked separation promise from inside this handler.
    void invoke<null>("cancel_stem_separation", { recordingId: id }).catch(() => {
      /* swallow: the cancellable promise below is the authoritative
         settle path — the IPC reject is a courtesy. */
    });
    const pending = pendingSeparations.get(id);
    if (pending !== undefined) {
      pendingSeparations.delete(id);
      pending.reject(new Error("Cancelled"));
    }
  },

  applyProgress: (p) => {
    // Receiver-closed-early contract: only honour the frame if the
    // store is in `separating` for this recording. A late-landing frame
    // that arrives after `cancel()` (or after `complete`) would
    // otherwise corrupt the FSM by re-mounting the progress bar.
    const cur = get().perRecording.get(p.recordingId);
    if (cur?.status !== "separating") return;

    // Hot-path percent: write to the ref bus directly. React DOES NOT
    // re-render for this — the panel's rAF loop reads the ref and
    // updates the `<progress>` element imperatively.
    writeStemPercent(p.recordingId, p.percent);

    // Stage transitions DO trigger a Zustand update so the label can
    // re-render and the polite live region can announce. Cheap because
    // a single recording emits at most 5 stage changes per separation.
    const prevActive = get().activeProgress;
    if (prevActive?.recordingId !== p.recordingId || prevActive.stage !== p.stage) {
      const stageCopy = stageDisplayLabel(p.stage);
      set((s) => ({
        activeProgress: { recordingId: p.recordingId, stage: p.stage },
        liveStatus: stageCopy,
        liveStatusByRecording: setLiveStatus(s.liveStatusByRecording, p.recordingId, stageCopy),
      }));
    }
  },

  __resetForTest: () => {
    pendingSeparations.clear();
    stemPercentByRecording.clear();
    set({
      perRecording: new Map(),
      activeProgress: null,
      liveStatus: null,
      liveStatusByRecording: new Map(),
    });
  },
}));

/** Stable idle sentinel — returning a fresh object literal from a Zustand
 *  selector on every render would trip React's getSnapshot caching guard
 *  ("The result of getSnapshot should be cached") and force an infinite
 *  re-render loop. We freeze a single shared instance so the selector's
 *  identity is stable across calls. */
const IDLE_SENTINEL: StemSeparationState = Object.freeze({ status: "idle" });

/** Convenience selector: lookup the state slot for a recording id, or
 *  return the shared idle sentinel if none. The panel uses this to keep
 *  its render branch logic flat. */
export function selectStemState(state: StemsState, id: RecordingId): StemSeparationState {
  return state.perRecording.get(id) ?? IDLE_SENTINEL;
}
