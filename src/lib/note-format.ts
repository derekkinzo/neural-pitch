// Equal-tempered note-name formatter.
//
// A pure function so React components can call it inline without effects.
//

const SHARP_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"] as const;
const ACCIDENTAL: ReadonlyArray<"" | "#"> = ["", "#", "", "#", "", "", "#", "", "#", "", "#", ""];

export type Accidental = "" | "#" | "b";

export interface NoteName {
  /** "A", "C", etc. — the natural-letter component. */
  letter: string;
  /** "" for naturals, "#" for sharps. (Flats are not emitted; the formatter always returns the sharp spelling.) */
  accidental: Accidental;
  /** Octave number (scientific pitch notation). */
  octave: number;
  /** Signed cents from the nearest equal-tempered note at the given a4. */
  cents: number;
}

/** Convert a frequency in Hz to its nearest equal-tempered note at `a4Hz`. */
export function hzToNote(hz: number, a4Hz: number): NoteName {
  if (!Number.isFinite(hz) || hz <= 0) {
    return { letter: "—", accidental: "", octave: 0, cents: 0 };
  }
  // MIDI 69 == A4. Solve midi = 69 + 12 * log2(hz/a4Hz).
  const midiFloat = 69 + 12 * Math.log2(hz / a4Hz);
  const midi = Math.round(midiFloat);
  const cents = (midiFloat - midi) * 100;
  const pc = ((midi % 12) + 12) % 12;
  const octave = Math.floor(midi / 12) - 1;
  const sharpName = SHARP_NAMES[pc] ?? "?";
  // Letter is the leading char; accidental is "" or "#".
  const letter = sharpName.charAt(0);
  const accidental: Accidental = ACCIDENTAL[pc] ?? "";
  return { letter, accidental, octave, cents };
}

/** Compute the equal-tempered Hz value for a target MIDI at the given a4. */
export function midiToHz(midi: number, a4Hz: number): number {
  return a4Hz * Math.pow(2, (midi - 69) / 12);
}

/** Stable display string, e.g. "A4", "C#5", or "—" for the silent state. */
export function formatNoteShort(note: NoteName): string {
  if (note.letter === "—") return "—";
  return `${note.letter}${note.accidental}${note.octave}`;
}

/** Stable display string for a MIDI number at the given `a4Hz`, e.g. "A4". */
export function formatMidiNote(midi: number, a4Hz: number): string {
  const hz = midiToHz(midi, a4Hz);
  return formatNoteShort(hzToNote(hz, a4Hz));
}

// ---------------------------------------------------------------------------
// Solfege rendering — ear-training subsystem.
//
// Two modes are supported beyond the default letter names:
//   - movable-do: solfege relative to the active drill's tonic. Tests rely
//     on the perfect-fifth radio reading "Sol" when the drill's tonic is
//     C and the mode is "movable-do".
//   - fixed-do:   solfege anchored to C, regardless of tonic. Same syllable
//     table; the relative-to-C semitone is the index.
//
// The drill UI passes its own tonic into the formatter so a
// transposing drill (or a movable-do tonic drift) does not require a
// global setting change.

/** Solfege syllables for the 12 chromatic pitch-classes. Sharp variants
 *  use the canonical "raised" form (Di, Ri, Fi, Si, Li). */
const SOLFEGE_SYLLABLES = [
  "Do",
  "Di",
  "Re",
  "Ri",
  "Mi",
  "Fa",
  "Fi",
  "Sol",
  "Si",
  "La",
  "Li",
  "Ti",
] as const;

// Single source of truth lives in `src/types/settings.ts`; both the
// drill UI and the settings drawer read from there.
import type { NoteLabelMode } from "@/types/settings";
export type { NoteLabelMode };

/**
 * Format a MIDI note for the given mode.
 *
 *   - "letter"      → "A4"  (scientific pitch notation via formatMidiNote)
 *   - "movable-do"  → solfege syllable relative to `tonicMidi` (drops octave)
 *   - "fixed-do"    → solfege syllable relative to C  (drops octave)
 *
 * Octave numbers are intentionally dropped for the solfege modes — solfege
 * is a pitch-class system; appending an octave would invent a convention
 * that does not exist in the literature. Drill UIs that need both the
 * syllable and the register render them side by side.
 */
export function formatNoteForMode(
  midi: number,
  a4Hz: number,
  mode: NoteLabelMode,
  tonicMidi: number,
): string {
  if (mode === "letter") return formatMidiNote(midi, a4Hz);
  const reference = mode === "movable-do" ? tonicMidi : 0; // C is MIDI 0 mod 12
  const offset = (((midi - reference) % 12) + 12) % 12;
  return SOLFEGE_SYLLABLES[offset] ?? "?";
}

/**
 * Format an interval (in semitones) for the given mode. The drill UI
 * builds the choice grid from a list of semitone values; the letter mode
 * maps them onto the canonical interval-quality token (m2..P8) while the
 * solfege modes resolve the syllable for tonic + interval.
 *
 * `tonicMidi` is consulted only by the movable-do mode; fixed-do uses C
 * regardless. Letter mode ignores both numerics — the token is the
 * semitone count's standard label.
 */
const INTERVAL_QUALITY_LABELS = [
  "P1", // 0 semitones
  "m2",
  "M2",
  "m3",
  "M3",
  "P4",
  "TT",
  "P5",
  "m6",
  "M6",
  "m7",
  "M7",
  "P8",
] as const;

export function formatIntervalLabel(
  semitones: number,
  mode: NoteLabelMode,
  tonicMidi: number,
): string {
  if (mode === "letter") {
    if (semitones < 0 || semitones >= INTERVAL_QUALITY_LABELS.length) {
      return `${semitones}st`;
    }
    return INTERVAL_QUALITY_LABELS[semitones] ?? `${semitones}st`;
  }
  // For solfege modes the row labels render the destination syllable —
  // the tonic is implied by the prompt's first note. The `a4Hz`
  // parameter is irrelevant in solfege modes because
  // `formatNoteForMode` short-circuits to a mod-12 syllable lookup;
  // we pass the standard reference rather than thread the live
  // setting through the call.
  return formatNoteForMode(tonicMidi + semitones, SOLFEGE_A4_REFERENCE_HZ, mode, tonicMidi);
}

/** Standard reference for the solfege branch of `formatIntervalLabel`.
 *  Unused by the underlying syllable lookup (mod-12 only) but threaded
 *  through `formatNoteForMode` to satisfy the shared signature. */
const SOLFEGE_A4_REFERENCE_HZ = 440;
