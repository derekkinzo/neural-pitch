// Training — wire-format types for the Phase 4 ear-training drill subsystem.
//
// Mirrors `src/types/transcription.ts` in shape: lightweight per-attempt
// records (`DrillAttempt`) feed the landing-screen "last attempt" stats;
// the heavy per-frame `MatchUpdate` flows through a dedicated channel
// the page-side store subscribes to. Field names are camelCase on the
// TS side; the IPC boundary maps from snake_case Rust per the existing
// `transcription.ts` convention.
//
// Drill IDs are an enum-as-string union — exhaustively switched in the
// landing-screen card list and the per-drill router so adding a sixth
// drill surfaces a compile-time gap.

/** Stable identifier for each drill kind. */
export type DrillId = "intervals" | "chords" | "scales" | "sight-singing" | "tuning";

/** Display metadata for a single drill card on the Training landing. */
export interface Drill {
  readonly id: DrillId;
  readonly title: string;
  readonly description: string;
}

/** A single completed drill attempt — one row in the persisted history. */
export interface DrillAttempt {
  readonly id: string;
  readonly drillId: DrillId;
  readonly startedAt: number;
  readonly completedAt: number;
  readonly totalPrompts: number;
  readonly correctCount: number;
  readonly accuracy: number;
}

/** A single note in a sight-singing target melody. */
export interface MelodyNote {
  readonly midi: number;
  readonly startMs: number;
  readonly durationMs: number;
}

export interface Melody {
  readonly id: string;
  /** Tonic MIDI note for movable-do solfege rendering. */
  readonly tonicMidi: number;
  readonly notes: readonly MelodyNote[];
}

/**
 * Per-frame match update emitted by `start_drill_match` over a
 * `Channel<MatchUpdate>`. The page-side store writes incoming updates
 * to `liveMatch` and the KaraokeRibbon repaints in its rAF loop.
 */
export interface MatchUpdate {
  readonly tMs: number;
  readonly targetMidi: number;
  readonly currentMidi: number;
  readonly centsOffset: number;
  readonly inTune: boolean;
  readonly barIndex: number;
  /** True on the final frame so the consumer can drain its queue. */
  readonly ended: boolean;
}

/** Live drill session — kept as a separate type so a future "resume" path
 *  can persist mid-flight progress without rewriting the attempt schema. */
export interface DrillSession {
  readonly drillId: DrillId;
  readonly startedAt: number;
  readonly totalPrompts: number;
  readonly answered: number;
  readonly correctCount: number;
}

// Re-export the canonical `NoteLabelMode` from the settings module so
// drill consumers and settings consumers see exactly the same type.
// Single source of truth: `src/types/settings.ts`.
export { NOTE_LABEL_MODES, type NoteLabelMode } from "@/types/settings";

/** Stable display metadata for the five built-in drills. The order here
 *  IS the visual order on the landing screen — re-arranging is a deliberate
 *  product decision. */
export const DRILLS: ReadonlyArray<Drill> = [
  {
    id: "intervals",
    title: "Intervals",
    description: "Identify the interval between two notes — m2 through P8.",
  },
  {
    id: "chords",
    title: "Chord quality",
    description: "Triads and seventh chords — Major, Minor, Dim, Aug, dom7, maj7, min7.",
  },
  {
    id: "scales",
    title: "Scale ID",
    description: "Recognise the seven church modes from a single ascending pattern.",
  },
  {
    id: "sight-singing",
    title: "Sight-singing",
    description: "Match the on-screen melody with your voice. The ribbon lights up in tune.",
  },
  {
    id: "tuning",
    title: "Tuning practice",
    description: "Hold a single sustained pitch within ten cents of the target for five seconds.",
  },
];
