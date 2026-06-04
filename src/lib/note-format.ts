// Equal-tempered note-name formatter.
//
// A pure function so React components can call it inline without effects.
// The Phase-4 movable-do solfège formatter (DESIGN.md §13.5, ADR-0004) will
// drop into the same shape behind a `Formatter` interface.
//
// Cross-references:
//   docs/adr/0004-default-note-name-system-english-with-formatter-trait.md
//   docs/design/DESIGN.md §7 (NoteDisplay surface)

const SHARP_NAMES = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"] as const;
const ACCIDENTAL: ReadonlyArray<"" | "#"> = ["", "#", "", "#", "", "", "#", "", "#", "", "#", ""];

export type Accidental = "" | "#" | "b";

export interface NoteName {
  /** "A", "C", etc. — the natural-letter component. */
  letter: string;
  /** "" for naturals, "#" for sharps. (Phase 1.2 always returns sharps.) */
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
