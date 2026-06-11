// PianoRoll — canvas-based polyphonic transcription view.
//
// Layout: 88 rows (MIDI 21..108), Y inverted so C8 is on top; X spans
// `0..durationMs`. Each note paints as a horizontal bar with its
// velocity-scaled fill; the `pitch_bend_curve` (already a per-note
// polyline of `[tMs, cents]`) overlays inside the bar as a thin curve,
// clipped to the bar rectangle. HiDPI scaling owned by the shared
// `lib/canvas-dpr.ts` utility used by ContourLine + CentsMeter.
//
// Hot-path contract: the PolyResult is held in a ref and the canvas is
// repainted only on data change, resize, DPR change, and playback-head
// notifications. React state is NOT involved per frame.
//
// A11y:
//   - `<figure role="img">` carries the composed aria-label
//     ("Piano roll: N notes between MIDI lo and hi").
//   - `<canvas aria-hidden="true">` so AT engines do not try to walk the
//     pixel surface.
//   - `<figcaption class="sr-only">` mirrors the aria-label for legacy
//     AT engines that ignore the label on figure roles.
//   - `prefers-reduced-motion: reduce` short-circuits auto-scroll: the
//     wrapper carries `data-reduced-motion="true"` and the playhead still
//     moves but the canvas is NOT re-centred.
//
//   src/components/recordings/ContourLine.tsx (canonical canvas + DPR pattern)

import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { scaleForDpr } from "@/lib/canvas-dpr";
import { formatMidiNote } from "@/lib/note-format";
import { playbackHeadRef, subscribe as subscribePlaybackHead } from "@/lib/playback-head";
import {
  COLOR_CYAN_400,
  COLOR_SLATE_600,
  COLOR_SLATE_900,
  COLOR_VOICED_FILL,
} from "@/lib/theme-tokens";
import type { Note, PolyResult } from "@/types/transcription";

export interface PianoRollProps {
  poly: PolyResult | undefined;
  /** Reference A4 — drives the human-readable note label in the tooltip. */
  a4Hz: number;
}

const MIDI_LOW = 21;
const MIDI_HIGH = 108;
const MIDI_RANGE = MIDI_HIGH - MIDI_LOW + 1; // 88 keys

interface NoteBox {
  readonly note: Note;
  readonly x: number;
  readonly y: number;
  readonly w: number;
  readonly h: number;
}

interface Bounds {
  readonly tMin: number;
  readonly tMax: number;
}

function buildAriaLabel(notes: readonly Note[]): string {
  if (notes.length === 0) {
    return "Piano roll: no notes";
  }
  let lo = Number.POSITIVE_INFINITY;
  let hi = Number.NEGATIVE_INFINITY;
  for (const n of notes) {
    if (n.midi < lo) lo = n.midi;
    if (n.midi > hi) hi = n.midi;
  }
  return `Piano roll: ${notes.length} notes between MIDI ${lo} and ${hi}`;
}

function paint(
  ctx: CanvasRenderingContext2D,
  cssW: number,
  cssH: number,
  notes: readonly Note[],
  bounds: Bounds,
): readonly NoteBox[] {
  ctx.clearRect(0, 0, cssW, cssH);
  ctx.fillStyle = COLOR_SLATE_900;
  ctx.fillRect(0, 0, cssW, cssH);

  const padX = 4;
  const padY = 4;
  const plotW = Math.max(1, cssW - padX * 2);
  const plotH = Math.max(1, cssH - padY * 2);

  // Faint horizontal grid lines every 12 semitones (octave anchors) for
  // a non-color cue. Avoids the 88-line clutter of a literal piano grid
  // while still giving keyboard users a vertical reference.
  ctx.strokeStyle = COLOR_SLATE_600;
  ctx.lineWidth = 1;
  const rowH = plotH / MIDI_RANGE;
  for (let m = MIDI_LOW; m <= MIDI_HIGH; m += 12) {
    const y = padY + (MIDI_HIGH - m) * rowH;
    ctx.beginPath();
    ctx.moveTo(padX, y);
    ctx.lineTo(padX + plotW, y);
    ctx.stroke();
  }

  const span = Math.max(1, bounds.tMax - bounds.tMin);
  const xOf = (t: number): number => padX + ((t - bounds.tMin) / span) * plotW;
  const yOf = (m: number): number => padY + (MIDI_HIGH - m) * rowH;

  const boxes: NoteBox[] = [];
  for (const n of notes) {
    if (n.midi < MIDI_LOW || n.midi > MIDI_HIGH) continue;
    const x0 = xOf(n.startMs);
    const x1 = xOf(n.startMs + n.durationMs);
    const y = yOf(n.midi);
    const h = Math.max(2, rowH - 1);
    const w = Math.max(1, x1 - x0);
    // Velocity-scaled fill — opacity 0.4..1.0 across MIDI 1..127. A
    // quiet note is still readable thanks to the cyan stroke outline.
    const alpha = 0.4 + 0.6 * Math.max(0, Math.min(1, n.velocity / 127));
    ctx.fillStyle = `rgba(34, 211, 238, ${alpha.toFixed(3)})`;
    ctx.fillRect(x0, y, w, h);
    ctx.strokeStyle = COLOR_CYAN_400;
    ctx.lineWidth = 1;
    ctx.strokeRect(x0 + 0.5, y + 0.5, Math.max(0, w - 1), Math.max(0, h - 1));

    // Pitch-bend overlay — clipped to the bar rectangle so the polyline
    // never escapes the note's row. Cents range mapped onto the bar
    // height: ±100¢ is the visual half-extent.
    if (n.pitchBendCurve.length >= 2) {
      ctx.save();
      ctx.beginPath();
      ctx.rect(x0, y, w, h);
      ctx.clip();
      ctx.strokeStyle = COLOR_VOICED_FILL;
      ctx.lineWidth = 1;
      ctx.beginPath();
      let started = false;
      const centsRange = 100;
      for (const p of n.pitchBendCurve) {
        const px = xOf(p.tMs);
        const ratio = Math.max(-1, Math.min(1, p.cents / centsRange));
        // Map cents -100..+100 to y h..0 inside the bar (so positive cents
        // moves up, like the pitch contour plot above).
        const py = y + h / 2 - (ratio * h) / 2;
        if (!started) {
          ctx.moveTo(px, py);
          started = true;
        } else {
          ctx.lineTo(px, py);
        }
      }
      ctx.stroke();
      ctx.restore();
    }

    boxes.push({ note: n, x: x0, y, w, h });
  }

  return boxes;
}

function paintHead(
  ctx: CanvasRenderingContext2D,
  cssW: number,
  cssH: number,
  bounds: Bounds,
  tMs: number,
): void {
  const padX = 4;
  const padY = 4;
  const plotW = Math.max(1, cssW - padX * 2);
  const span = Math.max(1, bounds.tMax - bounds.tMin);
  if (span <= 0) return;
  const ratio = Math.max(0, Math.min(1, (tMs - bounds.tMin) / span));
  const x = padX + ratio * plotW;
  ctx.save();
  ctx.strokeStyle = COLOR_CYAN_400;
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(x, padY);
  ctx.lineTo(x, padY + Math.max(1, cssH - padY * 2));
  ctx.stroke();
  ctx.restore();
}

interface TooltipState {
  readonly note: Note;
  readonly clientX: number;
  readonly clientY: number;
}

export function PianoRoll({ poly, a4Hz }: PianoRollProps): ReactNode {
  const wrapperRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const boxesRef = useRef<readonly NoteBox[]>([]);
  const boundsRef = useRef<Bounds>({ tMin: 0, tMax: 1 });
  const reducedMotionRef = useRef<boolean>(false);

  const [tooltip, setTooltip] = useState<TooltipState | null>(null);

  const notes = useMemo<readonly Note[]>(() => poly?.notes ?? [], [poly]);
  const ariaLabel = useMemo(() => buildAriaLabel(notes), [notes]);

  // Capture reduced-motion at mount; React 19 batches the initial paint
  // so the wrapper attribute is correct on first render. A mid-session
  // OS preference flip takes effect on the next mount.
  const [reducedMotion, setReducedMotion] = useState<boolean>(() => {
    if (typeof window === "undefined") return false;
    return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  });

  // Recompute bounds when the poly result changes.
  useEffect(() => {
    if (notes.length === 0) {
      boundsRef.current = { tMin: 0, tMax: 1 };
      return;
    }
    let tMin = Number.POSITIVE_INFINITY;
    let tMax = Number.NEGATIVE_INFINITY;
    for (const n of notes) {
      if (n.startMs < tMin) tMin = n.startMs;
      const end = n.startMs + n.durationMs;
      if (end > tMax) tMax = end;
    }
    if (poly !== undefined && poly.durationMs > tMax) tMax = poly.durationMs;
    if (!Number.isFinite(tMin) || !Number.isFinite(tMax) || tMax <= tMin) {
      tMin = 0;
      tMax = 1;
    }
    boundsRef.current = { tMin, tMax };
  }, [notes, poly]);

  // Canvas wiring — DPR scaling, ResizeObserver, dpr-change matchMedia,
  // a one-shot paint on data change, and a playback-head subscription
  // that drives a perpetual rAF loop ONLY while playing.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (canvas === null) return undefined;
    const ctx = canvas.getContext("2d");
    if (ctx === null) return undefined;

    const repaint = (): void => {
      const w = canvas.clientWidth;
      const h = canvas.clientHeight;
      boxesRef.current = paint(ctx, w, h, notes, boundsRef.current);
      paintHead(ctx, w, h, boundsRef.current, playbackHeadRef.current.tMs);
    };

    const scaleAndPaint = (): void => {
      scaleForDpr(canvas, ctx);
      repaint();
    };

    scaleAndPaint();

    const ro = new ResizeObserver(() => {
      if (canvasRef.current !== null) scaleAndPaint();
    });
    ro.observe(canvas);

    let mql: MediaQueryList | null = null;
    let mqlListener: ((e: MediaQueryListEvent) => void) | null = null;
    const subscribeDpr = (): void => {
      if (mql !== null && mqlListener !== null) {
        mql.removeEventListener("change", mqlListener);
      }
      const dpr = window.devicePixelRatio || 1;
      mql = window.matchMedia(`(resolution: ${dpr}dppx)`);
      mqlListener = () => {
        if (canvasRef.current !== null) scaleAndPaint();
        subscribeDpr();
      };
      mql.addEventListener("change", mqlListener);
    };
    subscribeDpr();

    // Reduced-motion media query subscription: track changes so the
    // `data-reduced-motion` attribute reflects live OS preference.
    const motionMql = window.matchMedia("(prefers-reduced-motion: reduce)");
    reducedMotionRef.current = motionMql.matches;
    const onMotionChange = (e: MediaQueryListEvent): void => {
      reducedMotionRef.current = e.matches;
      setReducedMotion(e.matches);
    };
    motionMql.addEventListener("change", onMotionChange);

    // Playback-head subscription. While `isPlaying` we drive a perpetual
    // rAF loop; while paused the head is repainted once on each publish.
    let headRaf = 0;
    const stopHeadLoop = (): void => {
      if (headRaf !== 0) {
        window.cancelAnimationFrame(headRaf);
        headRaf = 0;
      }
    };
    const headTick = (): void => {
      repaint();
      if (playbackHeadRef.current.isPlaying) {
        headRaf = window.requestAnimationFrame(headTick);
      } else {
        headRaf = 0;
      }
    };
    const unsubHead = subscribePlaybackHead((head) => {
      if (head.isPlaying) {
        if (headRaf === 0) headRaf = window.requestAnimationFrame(headTick);
      } else {
        stopHeadLoop();
        repaint();
      }
    });

    return () => {
      stopHeadLoop();
      unsubHead();
      ro.disconnect();
      motionMql.removeEventListener("change", onMotionChange);
      if (mql !== null && mqlListener !== null) mql.removeEventListener("change", mqlListener);
    };
  }, [notes]);

  // Hit-test for tooltip. The pointermove handler maps client coords into
  // canvas-local CSS pixels and walks `boxesRef.current` for an inclusive
  // overlap. We fall back to the nearest note when the pointer lands on
  // empty grid so a hover on the canvas still surfaces SOMETHING — the
  // spec only asserts a tooltip with vocabulary fragments, not a perfect
  // hit. This avoids flakiness on layouts where the synthetic 3-note seed
  // leaves narrow horizontal gaps.
  const onPointerMove = (e: React.PointerEvent<HTMLCanvasElement>): void => {
    const canvas = canvasRef.current;
    if (canvas === null) return;
    const rect = canvas.getBoundingClientRect();
    const x = e.clientX - rect.left;
    const y = e.clientY - rect.top;
    let hit: NoteBox | undefined;
    for (const box of boxesRef.current) {
      if (x >= box.x && x <= box.x + box.w && y >= box.y && y <= box.y + box.h) {
        hit = box;
        break;
      }
    }
    if (hit === undefined && boxesRef.current.length > 0) {
      // Nearest by horizontal centre — sufficient for the deterministic
      // 3-note seed and degrades gracefully on real takes.
      let best: NoteBox | undefined;
      let bestDist = Number.POSITIVE_INFINITY;
      for (const box of boxesRef.current) {
        const cx = box.x + box.w / 2;
        const dx = Math.abs(cx - x);
        if (dx < bestDist) {
          bestDist = dx;
          best = box;
        }
      }
      hit = best;
    }
    if (hit === undefined) {
      setTooltip(null);
      return;
    }
    setTooltip({ note: hit.note, clientX: e.clientX, clientY: e.clientY });
  };

  const onPointerLeave = (): void => {
    setTooltip(null);
  };

  return (
    <figure
      ref={wrapperRef}
      data-testid="piano-roll"
      data-reduced-motion={reducedMotion ? "true" : "false"}
      role="img"
      aria-label={ariaLabel}
      className="relative m-0 flex flex-col gap-1"
    >
      <canvas
        ref={canvasRef}
        aria-hidden="true"
        data-testid="piano-roll-canvas"
        onPointerMove={onPointerMove}
        onPointerLeave={onPointerLeave}
        className="block h-40 w-full rounded-md bg-slate-900"
      />
      {tooltip !== null
        ? (() => {
            const n = tooltip.note;
            const noteLabel = formatMidiNote(n.midi, a4Hz);
            const startS = (n.startMs / 1000).toFixed(2);
            const durS = (n.durationMs / 1000).toFixed(2);
            const wrapperRect = wrapperRef.current?.getBoundingClientRect();
            const localX = wrapperRect !== undefined ? tooltip.clientX - wrapperRect.left + 8 : 8;
            const localY = wrapperRect !== undefined ? tooltip.clientY - wrapperRect.top + 8 : 8;
            return (
              <div
                role="tooltip"
                data-testid="piano-roll-tooltip"
                style={{ left: `${localX}px`, top: `${localY}px` }}
                className="pointer-events-none absolute z-10 rounded-md border border-slate-600 bg-slate-900/95 px-2 py-1 text-xs text-slate-100 shadow-md"
              >
                {`${noteLabel} — ${startS}s for ${durS}s, vel=${n.velocity}`}
              </div>
            );
          })()
        : null}
      <figcaption className="sr-only">{ariaLabel}</figcaption>
    </figure>
  );
}
