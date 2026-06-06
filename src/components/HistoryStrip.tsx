// HistoryStrip — ~3 s of cents-history scrolled across a canvas.
//
// Reads up to ~280 frames from the rAF ring (capacity 256, ~2.7 s @ 93 Hz).
// Honours `prefers-reduced-motion` per ADR-0006 by short-circuiting the rAF
// loop and rendering a static `<output role="meter">` of the current cents.
//
// Cross-references:
//   docs/design/DESIGN.md §1 (history strip), §4 (canvas pattern)

import { useEffect, useRef, useState, type ReactNode } from "react";
import { SILENT_PITCH, type PitchUpdate } from "@/types/pitch";
import { scaleForDpr } from "@/lib/canvas-dpr";
import type { RingBuffer } from "@/lib/ring";

export interface HistoryStripProps {
  ringRef: React.RefObject<RingBuffer<PitchUpdate>>;
}

const HISTORY_FRAMES = 256;
const RANGE = 50;

function paint(
  ctx: CanvasRenderingContext2D,
  cssW: number,
  cssH: number,
  ring: RingBuffer<PitchUpdate>,
): void {
  ctx.clearRect(0, 0, cssW, cssH);

  // Center axis.
  const midY = cssH / 2;
  ctx.strokeStyle = "#1e293b";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(0, midY);
  ctx.lineTo(cssW, midY);
  ctx.stroke();

  if (ring.length === 0) return;

  const stepX = cssW / HISTORY_FRAMES;
  ctx.strokeStyle = "#22d3ee";
  ctx.lineWidth = 2;
  ctx.beginPath();
  let started = false;
  ring.forEachLast(HISTORY_FRAMES, (u, i) => {
    if (!u.voiced) {
      started = false;
      return;
    }
    const cents = Math.max(-RANGE, Math.min(RANGE, u.smoothed_cents));
    const x = i * stepX;
    const y = midY - (cents / RANGE) * (cssH / 2 - 4);
    if (!started) {
      ctx.moveTo(x, y);
      started = true;
    } else {
      ctx.lineTo(x, y);
    }
  });
  ctx.stroke();
}

interface StaticReadout {
  voiced: boolean;
  cents: number;
}

export function HistoryStrip({ ringRef }: HistoryStripProps): ReactNode {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const [reducedMotion, setReducedMotion] = useState<boolean>(false);
  const [staticReadout, setStaticReadout] = useState<StaticReadout>({
    voiced: false,
    cents: 0,
  });

  useEffect(() => {
    const mql = window.matchMedia("(prefers-reduced-motion: reduce)");
    const apply = (): void => setReducedMotion(mql.matches);
    apply();
    mql.addEventListener("change", apply);
    return () => mql.removeEventListener("change", apply);
  }, []);

  useEffect(() => {
    if (reducedMotion) {
      // In reduced-motion mode we still want a coarse readout. Poll the ring
      // every 250 ms — far below 60 Hz and below any reasonable "animation".
      // Track `voiced` alongside `cents` so silence and "in tune at 0¢" are
      // not collapsed to the same on-screen value.
      const id = window.setInterval(() => {
        const u = ringRef.current?.peekLatest() ?? SILENT_PITCH;
        setStaticReadout({ voiced: u.voiced, cents: Math.round(u.smoothed_cents) });
      }, 250);
      return () => window.clearInterval(id);
    }
    const canvas = canvasRef.current;
    if (canvas === null) return undefined;
    const ctx = canvas.getContext("2d");
    if (ctx === null) return undefined;
    scaleForDpr(canvas, ctx);
    const ro = new ResizeObserver(() => {
      if (canvasRef.current !== null) scaleForDpr(canvasRef.current, ctx);
    });
    ro.observe(canvas);

    let raf = 0;
    const tick = (): void => {
      const ring = ringRef.current;
      if (ring !== null) paint(ctx, canvas.clientWidth, canvas.clientHeight, ring);
      raf = window.requestAnimationFrame(tick);
    };
    raf = window.requestAnimationFrame(tick);
    return () => {
      window.cancelAnimationFrame(raf);
      ro.disconnect();
    };
  }, [reducedMotion, ringRef]);

  if (reducedMotion) {
    const { voiced, cents } = staticReadout;
    const display = voiced ? `${cents > 0 ? "+" : ""}${cents} ¢` : "no signal";
    return (
      <output
        data-testid="history-strip-static"
        role="meter"
        aria-label="Pitch deviation history (static)"
        aria-valuemin={-RANGE}
        aria-valuemax={RANGE}
        // aria-valuenow is meaningless for the silent case; AT users get
        // the textual "no signal" via aria-valuetext instead.
        aria-valuenow={voiced ? cents : 0}
        aria-valuetext={voiced ? `${cents} cents` : "no signal"}
        data-state={voiced ? "voiced" : "silent"}
        className="block h-16 w-full rounded-md bg-slate-900 px-3 py-2 font-mono text-sm text-slate-200"
      >
        {display}
      </output>
    );
  }

  return (
    <canvas
      ref={canvasRef}
      data-testid="history-strip-canvas"
      aria-hidden="true"
      className="block h-16 w-full rounded-md bg-slate-900"
    />
  );
}
