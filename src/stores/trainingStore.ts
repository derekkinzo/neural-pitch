// Training store — Phase 4 ear-training subsystem.
//
// Slow-path Zustand state for the Training landing screen + drill flows.
// Hot-path data (the live PitchUpdate ring + the per-frame KaraokeRibbon
// repaint) does NOT pass through Zustand: KaraokeRibbon holds the live
// `liveMatch` in a ref-mirrored slot driven by `setLiveMatch` and walks the
// existing `usePitchStream` ring directly. Per-attempt history and the
// active drill session are stable enough that a Zustand subscription is
// fine.
//
// History persistence: each `recordAttempt(...)` writes the appended
// history into `localStorage` under a single key. There is no IPC for
// this — the history is local to the desktop install, mirrors the
// `Phase 4 — local-only` constraint, and survives reloads. A failed
// localStorage write degrades to in-memory only (e.g. private browsing
// mode); the drill flow itself never blocks on persistence.

import { create } from "zustand";
import type {
  Drill,
  DrillAttempt,
  DrillId,
  DrillSession,
  MatchUpdate,
  Melody,
} from "@/types/training";

const HISTORY_STORAGE_KEY = "neural-pitch.training.history.v1";
const HISTORY_MAX_ENTRIES = 256;

/** Wire-format envelope for a MatchUpdate frame. The Tier-5 mock and the
 *  Rust shell both emit snake_case; future shell-side renames to camelCase
 *  are absorbed by reading either spelling. Re-exported so the
 *  match-update channel listener can share the same type without
 *  redeclaring it. */
export interface WireMatchUpdate {
  tMs?: number;
  t_ms?: number;
  targetMidi?: number;
  target_midi?: number;
  currentMidi?: number;
  current_midi?: number;
  centsOffset?: number;
  cents_offset?: number;
  inTune?: boolean;
  in_tune?: boolean;
  barIndex?: number;
  bar_index?: number;
  ended?: boolean;
}

/** Normalise the channel payload into the camelCase TS shape. The Tier-5
 *  mock emits snake_case (mirrors the Rust serde wire-format), and a
 *  future shell-side rename to camelCase is absorbed by the same path. */
export function normaliseMatchUpdate(raw: WireMatchUpdate): MatchUpdate {
  return {
    tMs: raw.tMs ?? raw.t_ms ?? 0,
    targetMidi: raw.targetMidi ?? raw.target_midi ?? 0,
    currentMidi: raw.currentMidi ?? raw.current_midi ?? 0,
    centsOffset: raw.centsOffset ?? raw.cents_offset ?? 0,
    inTune: raw.inTune ?? raw.in_tune ?? false,
    barIndex: raw.barIndex ?? raw.bar_index ?? 0,
    ended: raw.ended ?? false,
  };
}

function loadHistoryFromStorage(): DrillAttempt[] {
  if (typeof window === "undefined" || typeof window.localStorage === "undefined") {
    return [];
  }
  try {
    const raw = window.localStorage.getItem(HISTORY_STORAGE_KEY);
    if (raw === null) return [];
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) return [];
    return parsed.filter(isAttempt) as DrillAttempt[];
  } catch {
    return [];
  }
}

function saveHistoryToStorage(history: readonly DrillAttempt[]): void {
  if (typeof window === "undefined" || typeof window.localStorage === "undefined") {
    return;
  }
  try {
    window.localStorage.setItem(HISTORY_STORAGE_KEY, JSON.stringify(history));
  } catch {
    /* swallow: private-browsing / quota errors degrade to in-memory only */
  }
}

function isAttempt(v: unknown): v is DrillAttempt {
  if (v === null || typeof v !== "object") return false;
  const o = v as Record<string, unknown>;
  return (
    typeof o["id"] === "string" &&
    typeof o["drillId"] === "string" &&
    typeof o["startedAt"] === "number" &&
    typeof o["completedAt"] === "number" &&
    typeof o["totalPrompts"] === "number" &&
    typeof o["correctCount"] === "number" &&
    typeof o["accuracy"] === "number"
  );
}

export interface TrainingState {
  /** Active drill session — null between drills. */
  currentSession: DrillSession | null;
  /** Active drill metadata (id, title, description). */
  currentDrill: Drill | null;
  /** Append-only attempt history. Persisted via localStorage. */
  history: DrillAttempt[];
  /** Latest MatchUpdate frame from the Channel listener. The KaraokeRibbon
   *  consumes this for its rAF repaint; the spec asserts the figure's
   *  aria-label recomputes when this slot changes. */
  liveMatch: MatchUpdate | null;
  /** Active sight-singing melody. Set by `startSightSinging(melody)` so the
   *  drill component can paint target bars without re-fetching. */
  activeMelody: Melody | null;
}

export interface TrainingActions {
  /** Begin a new drill session. Resets `currentSession` counters. */
  beginSession: (drill: Drill, totalPrompts: number) => void;
  /** Record a single answer; advances the session counters. */
  scoreAnswer: (a: { correct: boolean }) => void;
  /** Finalise the active session, append to history, and clear live state. */
  completeSession: () => DrillAttempt | null;
  /** Mid-session abort. Drops the session without persisting an attempt. */
  abortSession: () => void;
  /** Set the active sight-singing melody. */
  setActiveMelody: (melody: Melody | null) => void;
  /** Push a fresh MatchUpdate frame. Guarded against the receiver tearing
   *  down before the test pushes the final frame — a no-op when nothing is
   *  listening (handled by the consumer's `cancelled` sentinel). */
  setLiveMatch: (m: MatchUpdate | null) => void;
  /** Replace the entire history. Used by the harness to seed the landing
   *  and by the IPC-hydrate path on Training mount. The optional
   *  `{persist: false}` flag skips the localStorage write so a stale or
   *  empty IPC response cannot wipe the on-disk client cache. */
  setHistory: (h: readonly DrillAttempt[], opts?: { persist?: boolean }) => void;
  /** Test-only reset between specs. */
  __resetForTest: () => void;
}

export type TrainingStore = TrainingState & TrainingActions;

export const useTrainingStore = create<TrainingStore>((set, get) => ({
  currentSession: null,
  currentDrill: null,
  history: loadHistoryFromStorage(),
  liveMatch: null,
  activeMelody: null,

  beginSession: (drill, totalPrompts) => {
    set({
      currentDrill: drill,
      currentSession: {
        drillId: drill.id,
        startedAt: Date.now(),
        totalPrompts,
        answered: 0,
        correctCount: 0,
      },
      liveMatch: null,
    });
  },

  scoreAnswer: ({ correct }) => {
    const session = get().currentSession;
    if (session === null) return;
    set({
      currentSession: {
        ...session,
        answered: session.answered + 1,
        correctCount: session.correctCount + (correct ? 1 : 0),
      },
    });
  },

  completeSession: () => {
    const session = get().currentSession;
    const drill = get().currentDrill;
    if (session === null || drill === null) return null;
    const completedAt = Date.now();
    const totalPrompts = session.totalPrompts;
    const correctCount = session.correctCount;
    const accuracy = totalPrompts > 0 ? correctCount / totalPrompts : 0;
    const attempt: DrillAttempt = {
      id: `att-${drill.id}-${completedAt.toString(36)}`,
      drillId: drill.id,
      startedAt: session.startedAt,
      completedAt,
      totalPrompts,
      correctCount,
      accuracy,
    };
    const next = [attempt, ...get().history].slice(0, HISTORY_MAX_ENTRIES);
    set({
      currentSession: null,
      currentDrill: null,
      history: next,
      liveMatch: null,
      activeMelody: null,
    });
    saveHistoryToStorage(next);
    return attempt;
  },

  abortSession: () => {
    set({
      currentSession: null,
      currentDrill: null,
      liveMatch: null,
      activeMelody: null,
    });
  },

  setActiveMelody: (melody) => set({ activeMelody: melody }),

  setLiveMatch: (m) => set({ liveMatch: m }),

  setHistory: (h, opts) => {
    const next = [...h];
    set({ history: next });
    if (opts?.persist !== false) {
      saveHistoryToStorage(next);
    }
  },

  __resetForTest: () => {
    set({
      currentSession: null,
      currentDrill: null,
      history: [],
      liveMatch: null,
      activeMelody: null,
    });
    saveHistoryToStorage([]);
  },
}));

/** Selector: the most recent attempt for a given drill, or `null`. */
export function selectLatestAttempt(state: TrainingState, drillId: DrillId): DrillAttempt | null {
  for (const attempt of state.history) {
    if (attempt.drillId === drillId) return attempt;
  }
  return null;
}
