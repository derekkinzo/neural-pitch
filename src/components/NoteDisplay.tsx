// NoteDisplay — the giant chromatic note + Hz readout.
//
// Reads the rAF ring directly via the `ringRef` prop and writes to DOM
// nodes through refs:
//   - .note-letter      : "A", "C#", etc. + octave subscript (visual)
//   - .note-hz          : "440.00 Hz" (visual)
//   - data-testid="note-aria-live" : invisible AT-only mirror that only
//     speaks the meaningful note label. The visible glyph block is NOT
//     `aria-live` so silence transitions ("—") and cold-start states do
//     not announce dashes.
//
// React state is only used for `data-state` (in-tune / sharp / flat / silent)
// and the live-region textContent — both updated on note transitions, not
// per frame. The per-frame visual text updates happen through `textContent`
// writes, avoiding React reconciliation on the hot path.
//
// The slow `tunerStore.lastVoicedNoteLabel` mirror is written exclusively
// by `usePitchStream` so the AT label and visible glyph cannot diverge near
// the cents-50 boundary (see review feedback "ui.bug — duplicate writers").
//
// Cross-references:
//   docs/design/DESIGN.md §5 (ARIA strategy)

import { useEffect, useRef, type ReactNode } from "react";
import { hzToNote, formatNoteShort, type NoteName } from "@/lib/note-format";
import { useSettingsStore } from "@/stores/settingsStore";
import { SILENT_PITCH, type PitchUpdate } from "@/types/pitch";
import type { RingBuffer } from "@/lib/ring";

export interface NoteDisplayProps {
  ringRef: React.RefObject<RingBuffer<PitchUpdate>>;
}

const SHARP = "♯";
const FLAT = "♭";

function accidentalGlyph(note: NoteName): string {
  if (note.accidental === "#") return SHARP;
  if (note.accidental === "b") return FLAT;
  return "";
}

function classifyState(update: PitchUpdate): "silent" | "in-tune" | "sharp" | "flat" {
  if (!update.voiced) return "silent";
  const cents = update.smoothed_cents;
  if (cents > 5) return "sharp";
  if (cents < -5) return "flat";
  return "in-tune";
}

export function NoteDisplay({ ringRef }: NoteDisplayProps): ReactNode {
  const rootRef = useRef<HTMLDivElement | null>(null);
  const letterRef = useRef<HTMLSpanElement | null>(null);
  const accidentalRef = useRef<HTMLSpanElement | null>(null);
  const octaveRef = useRef<HTMLSpanElement | null>(null);
  const hzRef = useRef<HTMLOutputElement | null>(null);
  const liveRef = useRef<HTMLSpanElement | null>(null);

  useEffect(() => {
    let raf = 0;
    let lastLabel = "";
    let lastState = "";
    let hasSeenVoiced = false;

    const tick = (): void => {
      const ring = ringRef.current;
      const u = ring?.peekLatest() ?? SILENT_PITCH;
      const a4 = useSettingsStore.getState().a4Hz;
      const note = hzToNote(u.voiced ? u.f0_hz : 0, a4);
      const label = u.voiced ? formatNoteShort(note) : "—";
      if (label !== lastLabel) {
        lastLabel = label;
        if (letterRef.current) letterRef.current.textContent = note.letter;
        if (accidentalRef.current) accidentalRef.current.textContent = accidentalGlyph(note);
        if (octaveRef.current) octaveRef.current.textContent = u.voiced ? String(note.octave) : "";
        // The dedicated AT-only live region is updated *only* on voiced
        // transitions — silence and cold-start are rendered as empty
        // textContent so AT does not announce "em dash" or "dash dash".
        // First voiced frame unlocks announcements.
        if (liveRef.current) {
          if (u.voiced) {
            hasSeenVoiced = true;
            liveRef.current.textContent = label;
          } else if (hasSeenVoiced) {
            liveRef.current.textContent = "";
          }
        }
      }
      const hzText = u.voiced ? `${u.f0_hz.toFixed(2)} Hz` : "—";
      if (hzRef.current && hzRef.current.textContent !== hzText) {
        hzRef.current.textContent = hzText;
      }
      const state = classifyState(u);
      if (state !== lastState) {
        lastState = state;
        rootRef.current?.setAttribute("data-state", state);
      }
      raf = window.requestAnimationFrame(tick);
    };
    raf = window.requestAnimationFrame(tick);
    return () => window.cancelAnimationFrame(raf);
  }, [ringRef]);

  return (
    <div
      ref={rootRef}
      data-testid="note-display"
      data-state="silent"
      className="flex flex-col items-center gap-2"
    >
      <div className="flex items-baseline gap-1 text-slate-100">
        <span
          ref={letterRef}
          data-testid="note-letter"
          aria-hidden="true"
          className="text-[10rem] font-bold leading-none tracking-tight text-slate-50"
        >
          —
        </span>
        <span
          ref={accidentalRef}
          data-testid="note-accidental"
          aria-hidden="true"
          className="text-[5rem] font-semibold text-amber-400"
        ></span>
        <sub
          ref={octaveRef}
          data-testid="note-octave"
          aria-hidden="true"
          className="text-[2.5rem] font-medium text-slate-300"
        ></sub>
      </div>
      <output
        ref={hzRef}
        data-testid="note-hz"
        aria-hidden="true"
        className="font-mono text-base text-slate-400"
      >
        —
      </output>
      {/* AT-only live region. role="status" + empty-string textContent for
          silence/cold-start ensures screen readers stay quiet until the
          first meaningful note arrives. */}
      <span
        ref={liveRef}
        data-testid="note-aria-live"
        role="status"
        aria-live="polite"
        aria-atomic="true"
        className="sr-only"
      ></span>
    </div>
  );
}
