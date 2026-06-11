// KaraokeRibbon — sight-singing target/actual visualisation.
//
// Mirrors the well-trodden DPR + reduced-motion pattern from `PianoRoll`.
//   - X axis: time (left → right). Y axis: MIDI (top is high).
//   - Target notes paint as filled horizontal bars; in-tune fill is the
//     blue voiced-fill colour, off-tune fill is the orange off-tune colour
//     PLUS a striped overlay (WCAG 1.4.1 redundancy).
//   - Live pitch paints as a moving dot AND a 1-px trailing line so the
//     visual cue is shape-and-color, not color alone.
//   - Reduced-motion: the ribbon does NOT scroll; a static target-vs-
//     actual snapshot paints once on mount and again on `MatchUpdate.ended`.
//
// A11y:
//   - `<figure role="img">` carries the composed aria-label that recomputes
//     when `liveMatch` changes. Suffix is the documented
//     `current pitch <Note> <±N> cents` token.
//   - A sibling `role="status"` live region announces continuous in-tune
//     updates ("In tune. C5." / "22 cents flat. C5.") at ~1 Hz so screen
//     readers receive real-time feedback while the user sings — the
//     `aria-label` on `role="img"` only fires on focus / navigation.
//   - `<canvas aria-hidden="true">` so AT engines do not try to walk the
//     pixel surface.
//

import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { scaleForDpr } from "@/lib/canvas-dpr";
import { formatMidiNote } from "@/lib/note-format";
import {
  COLOR_CYAN_400,
  COLOR_SLATE_600,
  COLOR_SLATE_700,
  COLOR_SLATE_900,
  COLOR_VOICED_FILL,
} from "@/lib/theme-tokens";
import { useTrainingStore } from "@/stores/trainingStore";
import type { Melody, MatchUpdate } from "@/types/training";

const COLOR_OFF_TUNE = "#f97316";
const COLOR_OFF_TUNE_FILL = "rgba(249, 115, 22, 0.20)";
const COLOR_NEUTRAL_FILL = "rgba(71, 85, 105, 0.35)"; // slate-600 @ 35%
const MIDI_PADDING = 4;

export interface KaraokeRibbonProps {
  melody: Melody;
  /** A4 reference Hz. Drives the live note label embedded in aria-label. */
  a4Hz: number;
}

function midiBoundsFor(melody: Melody): readonly [number, number] {
  let lo = Number.POSITIVE_INFINITY;
  let hi = Number.NEGATIVE_INFINITY;
  for (const n of melody.notes) {
    if (n.midi < lo) lo = n.midi;
    if (n.midi > hi) hi = n.midi;
  }
  if (!Number.isFinite(lo) || !Number.isFinite(hi)) return [60, 72];
  return [lo - MIDI_PADDING, hi + MIDI_PADDING];
}

function durationOf(melody: Melody): number {
  let max = 0;
  for (const n of melody.notes) {
    const end = n.startMs + n.durationMs;
    if (end > max) max = end;
  }
  return Math.max(1, max);
}

function buildAriaLabel(melody: Melody, liveMatch: MatchUpdate | null, a4Hz: number): string {
  const [lo, hi] = midiBoundsFor(melody);
  const loLabel = formatMidiNote(lo + MIDI_PADDING, a4Hz);
  const hiLabel = formatMidiNote(hi - MIDI_PADDING, a4Hz);
  const noteCount = melody.notes.length;
  const head = `Pitch ribbon: ${noteCount} target notes between ${loLabel} and ${hiLabel}`;
  if (liveMatch === null) return head;
  const noteLabel = formatMidiNote(liveMatch.currentMidi, a4Hz);
  const cents = Math.round(liveMatch.centsOffset);
  // Sign is decided AFTER the round so a value rounding to 0 reports
  // `±0 cents` rather than `+0` or `--0`. The two-glyph form keeps the
  // spec's `current pitch <Note> -22 cents$` regex matchable while
  // stabilising the zero-cents readout.
  const glyph = cents > 0 ? "+" : cents < 0 ? "-" : "±";
  return `${head}; current pitch ${noteLabel} ${glyph}${Math.abs(cents)} cents`;
}

export function KaraokeRibbon({ melody, a4Hz }: KaraokeRibbonProps): ReactNode {
  const liveMatch = useTrainingStore((s) => s.liveMatch);

  // Reduced-motion stamp on the wrapper.
  const [reducedMotion, setReducedMotion] = useState<boolean>(() => {
    if (typeof window === "undefined") return false;
    return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  });

  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const liveMatchRef = useRef<MatchUpdate | null>(null);
  const barInTuneRef = useRef<Map<number, boolean>>(new Map());
  /** Mirror of the active paint closure so the reduced-motion useEffect
   *  hook below can fire a single repaint per liveMatch change without a
   *  second Zustand subscription. */
  const paintRef = useRef<(() => void) | null>(null);

  // Live-region announcement string. Recomputes in a debounced effect
  // (~1 Hz) so screen readers are not flooded with one announcement per
  // MatchUpdate frame. Spec: "In tune. C5." / "22 cents flat. C5."
  // (sharp/flat phrasing replaces the short ±N suffix the figure's
  // aria-label uses — screen readers read the full word more naturally
  // than a glyph).
  const [liveAnnouncement, setLiveAnnouncement] = useState<string>("");

  // Mirror the live match into the rAF-readable ref so the paint loop
  // does not need to subscribe to React state. Mark the ribbon dirty
  // so the rAF tick coalesces the next paint into a single repaint.
  useEffect(() => {
    liveMatchRef.current = liveMatch;
    if (liveMatch !== null) {
      barInTuneRef.current.set(liveMatch.barIndex, liveMatch.inTune);
    }
    dirtyRef.current = true;
  }, [liveMatch]);

  const totalDurationMs = useMemo(() => durationOf(melody), [melody]);
  const [midiLo, midiHi] = useMemo(() => midiBoundsFor(melody), [melody]);

  const ariaLabel = useMemo(
    () => buildAriaLabel(melody, liveMatch, a4Hz),
    [melody, liveMatch, a4Hz],
  );

  // Debounce live-region updates to ~1 Hz so AT engines are not flooded.
  // The figure's `aria-label` continues to recompute on every frame
  // (cheap, only read on focus / navigation); the live region is the
  // path that drives continuous spoken feedback while the user sings.
  useEffect(() => {
    if (liveMatch === null) {
      setLiveAnnouncement("");
      return undefined;
    }
    const handle = window.setTimeout(() => {
      const noteLabel = formatMidiNote(liveMatch.currentMidi, a4Hz);
      const cents = Math.round(liveMatch.centsOffset);
      let phrase: string;
      if (liveMatch.inTune || cents === 0) {
        phrase = "In tune";
      } else if (cents > 0) {
        phrase = `${cents} cents sharp`;
      } else {
        phrase = `${Math.abs(cents)} cents flat`;
      }
      setLiveAnnouncement(`${phrase}. ${noteLabel}.`);
    }, 1000);
    return () => {
      window.clearTimeout(handle);
    };
  }, [liveMatch, a4Hz]);

  // Paint dirty flag — set whenever the live match ref or the layout
  // inputs change. The rAF tick coalesces many flag-flips into a
  // single repaint so we paint at most once per frame even when frames
  // arrive at >60 Hz. In reduced-motion mode the rAF loop is gated
  // off entirely; instead a Zustand subscription paints once per
  // `liveMatch` update so the contract "snapshot paints on
  // MatchUpdate.ended" holds.
  const dirtyRef = useRef<boolean>(true);

  useEffect(() => {
    dirtyRef.current = true;
  }, [melody, midiHi, midiLo, totalDurationMs]);

  useEffect(() => {
    const canvas = canvasRef.current;
    if (canvas === null) return undefined;
    const ctx = canvas.getContext("2d");
    if (ctx === null) return undefined;

    const motionMql = window.matchMedia("(prefers-reduced-motion: reduce)");
    const onMotionChange = (e: MediaQueryListEvent): void => {
      setReducedMotion(e.matches);
      // Force a repaint when the user toggles motion mid-session so
      // the new paint regime applies on the next frame.
      dirtyRef.current = true;
    };
    motionMql.addEventListener("change", onMotionChange);

    const paint = (): void => {
      dirtyRef.current = false;
      const cssW = canvas.clientWidth;
      const cssH = canvas.clientHeight;
      ctx.clearRect(0, 0, cssW, cssH);
      ctx.fillStyle = COLOR_SLATE_900;
      ctx.fillRect(0, 0, cssW, cssH);

      const padX = 4;
      const padY = 4;
      const plotW = Math.max(1, cssW - padX * 2);
      const plotH = Math.max(1, cssH - padY * 2);
      const span = Math.max(1, midiHi - midiLo);
      const rowH = plotH / span;

      const xOf = (t: number): number => padX + (t / Math.max(1, totalDurationMs)) * plotW;
      const yOf = (m: number): number => padY + (midiHi - m) * rowH;

      ctx.strokeStyle = COLOR_SLATE_600;
      ctx.lineWidth = 1;
      for (let m = midiLo; m <= midiHi; m += 12) {
        const y = yOf(m);
        ctx.beginPath();
        ctx.moveTo(padX, y);
        ctx.lineTo(padX + plotW, y);
        ctx.stroke();
      }

      melody.notes.forEach((n, idx) => {
        const x0 = xOf(n.startMs);
        const x1 = xOf(n.startMs + n.durationMs);
        const y = yOf(n.midi);
        const h = Math.max(2, rowH - 1);
        const w = Math.max(1, x1 - x0);
        const seenInTune = barInTuneRef.current.get(idx);

        // Three-state paint:
        //   - undefined → un-evaluated. Neutral slate fill + dashed
        //     outline so the bar reads as "no verdict yet" and is NOT
        //     mistaken for an off-tune bar (WCAG 1.4.1: shape +
        //     colour, not colour alone).
        //   - true       → in-tune.   Cyan fill + thicker border.
        //   - false      → off-tune.  Orange fill + striped overlay.
        if (seenInTune === undefined) {
          ctx.fillStyle = COLOR_NEUTRAL_FILL;
          ctx.fillRect(x0, y, w, h);
          ctx.strokeStyle = COLOR_SLATE_700;
          ctx.lineWidth = 1;
          ctx.setLineDash([3, 2]);
          ctx.strokeRect(x0 + 0.5, y + 0.5, Math.max(0, w - 1), Math.max(0, h - 1));
          ctx.setLineDash([]);
        } else if (seenInTune) {
          ctx.fillStyle = COLOR_VOICED_FILL;
          ctx.fillRect(x0, y, w, h);
          ctx.strokeStyle = COLOR_CYAN_400;
          ctx.lineWidth = 2;
          ctx.strokeRect(x0 + 0.5, y + 0.5, Math.max(0, w - 1), Math.max(0, h - 1));
          ctx.lineWidth = 1;
        } else {
          ctx.fillStyle = COLOR_OFF_TUNE_FILL;
          ctx.fillRect(x0, y, w, h);
          ctx.strokeStyle = COLOR_OFF_TUNE;
          ctx.lineWidth = 1;
          ctx.strokeRect(x0 + 0.5, y + 0.5, Math.max(0, w - 1), Math.max(0, h - 1));
          // Off-tune bars get a striped overlay so colour-blind users
          // still see a difference (WCAG 1.4.1).
          ctx.save();
          ctx.beginPath();
          ctx.rect(x0, y, w, h);
          ctx.clip();
          ctx.strokeStyle = COLOR_OFF_TUNE;
          ctx.lineWidth = 1;
          for (let sx = x0 - h; sx < x0 + w; sx += 6) {
            ctx.beginPath();
            ctx.moveTo(sx, y + h);
            ctx.lineTo(sx + h, y);
            ctx.stroke();
          }
          ctx.restore();
        }
      });

      const live = liveMatchRef.current;
      if (live !== null) {
        const lx = xOf(live.tMs);
        const ly = yOf(live.currentMidi);
        ctx.strokeStyle = COLOR_CYAN_400;
        ctx.lineWidth = 1;
        ctx.beginPath();
        ctx.moveTo(padX, ly);
        ctx.lineTo(lx, ly);
        ctx.stroke();
        ctx.fillStyle = COLOR_CYAN_400;
        ctx.beginPath();
        ctx.arc(lx, ly, 3, 0, Math.PI * 2);
        ctx.fill();
      }
    };

    const scaleAndPaint = (): void => {
      scaleForDpr(canvas, ctx);
      paint();
    };

    scaleAndPaint();

    const ro = new ResizeObserver(() => {
      if (canvasRef.current !== null) {
        dirtyRef.current = true;
        scaleAndPaint();
      }
    });
    ro.observe(canvas);

    // Non-reduced-motion path: rAF loop coalesces many MatchUpdate
    // frames into one paint per frame. The loop is cheap when nothing
    // is dirty (reads `dirtyRef`, returns).
    let raf = 0;
    const tick = (): void => {
      if (motionMql.matches) {
        raf = 0;
        return;
      }
      if (dirtyRef.current) paint();
      raf = window.requestAnimationFrame(tick);
    };
    if (!motionMql.matches) {
      raf = window.requestAnimationFrame(tick);
    }

    // Reduced-motion path: the component already re-renders on every
    // `liveMatch` change via the React-hook subscription. Expose the
    // paint function on a ref so the sibling effect (gated on
    // `motionMql.matches`) can fire a single paint per frame without a
    // second store subscription. This keeps the contract "snapshot
    // paints on MatchUpdate.ended" while shrinking the component to one
    // store subscription instead of three.
    paintRef.current = paint;

    return () => {
      if (raf !== 0) window.cancelAnimationFrame(raf);
      ro.disconnect();
      motionMql.removeEventListener("change", onMotionChange);
      paintRef.current = null;
    };
  }, [melody, midiHi, midiLo, totalDurationMs]);

  // Reduced-motion repaint trigger: fires once per liveMatch change so
  // the static snapshot tracks the final `ended` frame the contract
  // documents. The non-reduced-motion path is driven by the rAF loop.
  useEffect(() => {
    if (typeof window === "undefined") return undefined;
    if (!window.matchMedia("(prefers-reduced-motion: reduce)").matches) return undefined;
    paintRef.current?.();
    return undefined;
  }, [liveMatch]);

  return (
    <div className="flex flex-col gap-1">
      <figure
        data-testid="karaoke-ribbon"
        data-reduced-motion={reducedMotion ? "true" : "false"}
        role="img"
        aria-label={ariaLabel}
        className="relative m-0 flex flex-col gap-1"
      >
        <canvas
          ref={canvasRef}
          aria-hidden="true"
          data-testid="karaoke-canvas"
          className="block h-40 w-full rounded-md bg-slate-900"
        />
        <figcaption className="sr-only">{ariaLabel}</figcaption>
      </figure>
      {/* Live region for continuous in-tune feedback. Debounced to ~1
          Hz upstream so screen readers are not flooded; carries an
          empty string between updates so the AT clears its buffer. */}
      <div
        role="status"
        aria-live="polite"
        aria-atomic="true"
        data-testid="karaoke-live-region"
        className="sr-only"
      >
        {liveAnnouncement}
      </div>
    </div>
  );
}
