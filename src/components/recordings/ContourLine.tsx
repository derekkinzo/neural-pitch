// ContourLine — Phase 2.1 pitch-contour plot.
//
// Time on the x-axis, cents-off-from-median on the y-axis (clamped to ±100¢
// default, expanding only if the trace exceeds it). Voiced spans drawn as a
// connected polyline; unvoiced spans rendered as gaps (`ctx.beginPath()`
// restarted on each unvoiced→voiced transition).
//
// Reuses the CentsMeter pattern verbatim:
//   - useRef<HTMLCanvasElement> + useEffect with scaleForDpr() (DPR scaling,
//     ResizeObserver, plus `matchMedia("(resolution: <N>dppx)")` re-scale).
//   - rAF loop is used ONLY for one-shot redraws on data change. The contour
//     is static after analysis settles, so the rAF tick exits after one
//     paint per data update — different from the perpetual paint loop the
//     live tuner uses.
//   - `prefers-reduced-motion: reduce` paints the static plot snapshot in a
//     single synchronous call after layout (no fade-in, no progressive
//     draw — ADR-0006 visual-only-feedback contract).
//   - Downsampling: cap at ~500 frames per second of recording. Run
//     downsampling once on data arrival and stash the result in a ref so
//     the rAF redraw reads from it.
//
// ARIA: the wrapping `<div role="img">` carries the semantic role; the
// `<canvas>` itself is `aria-hidden="true"` (matches CentsMeter). The
// aria-label is composed from the AnalysisSummary numbers — we do NOT
// stream per-frame data into ARIA.
//
// Cross-references:
//   docs/design/DESIGN.md §7.5 (Phase 2.1 frontend additions)
//   docs/adr/0006-visual-only-feedback-prefers-reduced-motion.md
//   src/components/CentsMeter.tsx (canonical canvas + DPR + reduced-motion pattern)

import { useEffect, useMemo, useRef, type ReactNode } from "react";
import type { AnalysisSummary, ContourFrame, ContourResult } from "@/types/analysis";

export interface ContourLineProps {
  summary: AnalysisSummary | undefined;
  contour: ContourResult | undefined;
}

const DEFAULT_RANGE = 100; // ± cents — y-axis half-range
const FRAMES_PER_SECOND_CAP = 500;

const COLORS = {
  bg: "#0f172a", // slate-900
  axis: "#475569", // slate-600
  zero: "#22d3ee", // cyan-400
  voiced: "#22d3ee", // cyan-400 — primary contour
  voicedFill: "rgba(34, 211, 238, 0.12)",
} as const;

interface DrawableSegment {
  /** Frames belonging to a single voiced run. Unvoiced runs are NOT
   *  represented as segments — they show up as gaps between consecutive
   *  segments in the array. */
  readonly frames: readonly ContourFrame[];
}

/** Largest-Triangle-Three-Buckets downsampling. Preserves the visual peaks
 *  of a time series while dropping the bucketed midpoints. The bucket count
 *  is `target - 2` (first and last frames preserved verbatim). When the
 *  input is smaller than the target we fall through to the original list. */
function lttb(frames: readonly ContourFrame[], target: number): readonly ContourFrame[] {
  if (frames.length <= target || target < 3) return frames;
  const sampled: ContourFrame[] = [];
  const bucketSize = (frames.length - 2) / (target - 2);
  // Always keep the first frame.
  const first = frames[0];
  if (first === undefined) return frames;
  sampled.push(first);
  let a = 0;
  for (let i = 0; i < target - 2; i += 1) {
    const avgRangeStart = Math.floor((i + 1) * bucketSize) + 1;
    const avgRangeEnd = Math.min(frames.length, Math.floor((i + 2) * bucketSize) + 1);
    let avgT = 0;
    let avgC = 0;
    const avgRangeLen = avgRangeEnd - avgRangeStart;
    if (avgRangeLen <= 0) continue;
    for (let j = avgRangeStart; j < avgRangeEnd; j += 1) {
      const f = frames[j];
      if (f === undefined) continue;
      avgT += f.tMs;
      avgC += f.centsFromMedian;
    }
    avgT /= avgRangeLen;
    avgC /= avgRangeLen;

    const rangeOffs = Math.floor(i * bucketSize) + 1;
    const rangeTo = Math.floor((i + 1) * bucketSize) + 1;
    const aFrame = frames[a];
    if (aFrame === undefined) continue;
    let maxArea = -1;
    let nextA = rangeOffs;
    let chosen: ContourFrame | undefined;
    for (let j = rangeOffs; j < rangeTo; j += 1) {
      const f = frames[j];
      if (f === undefined) continue;
      const area =
        Math.abs(
          (aFrame.tMs - avgT) * (f.centsFromMedian - aFrame.centsFromMedian) -
            (aFrame.tMs - f.tMs) * (avgC - aFrame.centsFromMedian),
        ) * 0.5;
      if (area > maxArea) {
        maxArea = area;
        chosen = f;
        nextA = j;
      }
    }
    if (chosen !== undefined) sampled.push(chosen);
    a = nextA;
  }
  const last = frames[frames.length - 1];
  if (last !== undefined) sampled.push(last);
  return sampled;
}

/** Split a frame stream into voiced segments. Unvoiced runs become gaps
 *  between consecutive segments. */
function toSegments(frames: readonly ContourFrame[]): DrawableSegment[] {
  const out: DrawableSegment[] = [];
  let cur: ContourFrame[] = [];
  for (const f of frames) {
    if (f.voiced) {
      cur.push(f);
    } else if (cur.length > 0) {
      out.push({ frames: cur });
      cur = [];
    }
  }
  if (cur.length > 0) out.push({ frames: cur });
  return out;
}

function downsample(frames: readonly ContourFrame[], durationMs: number): readonly ContourFrame[] {
  const seconds = Math.max(1, Math.ceil(durationMs / 1000));
  const target = Math.min(frames.length, FRAMES_PER_SECOND_CAP * seconds);
  return lttb(frames, target);
}

function scaleForDpr(canvas: HTMLCanvasElement, ctx: CanvasRenderingContext2D): void {
  const dpr = window.devicePixelRatio || 1;
  const cssW = canvas.clientWidth;
  const cssH = canvas.clientHeight;
  canvas.width = Math.max(1, Math.round(cssW * dpr));
  canvas.height = Math.max(1, Math.round(cssH * dpr));
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
}

interface PaintBounds {
  readonly tMin: number;
  readonly tMax: number;
  readonly cMin: number;
  readonly cMax: number;
}

function computeBounds(frames: readonly ContourFrame[]): PaintBounds {
  if (frames.length === 0) {
    return { tMin: 0, tMax: 1, cMin: -DEFAULT_RANGE, cMax: DEFAULT_RANGE };
  }
  let tMin = Number.POSITIVE_INFINITY;
  let tMax = Number.NEGATIVE_INFINITY;
  let cMin = -DEFAULT_RANGE;
  let cMax = DEFAULT_RANGE;
  for (const f of frames) {
    if (f.tMs < tMin) tMin = f.tMs;
    if (f.tMs > tMax) tMax = f.tMs;
    if (f.voiced) {
      if (f.centsFromMedian < cMin) cMin = f.centsFromMedian;
      if (f.centsFromMedian > cMax) cMax = f.centsFromMedian;
    }
  }
  if (!Number.isFinite(tMin) || !Number.isFinite(tMax) || tMax <= tMin) {
    tMin = 0;
    tMax = 1;
  }
  return { tMin, tMax, cMin, cMax };
}

function paint(
  ctx: CanvasRenderingContext2D,
  cssW: number,
  cssH: number,
  segments: readonly DrawableSegment[],
  bounds: PaintBounds,
): void {
  ctx.clearRect(0, 0, cssW, cssH);
  // Plot background.
  ctx.fillStyle = COLORS.bg;
  ctx.fillRect(0, 0, cssW, cssH);

  const padX = 8;
  const padY = 8;
  const plotW = Math.max(1, cssW - padX * 2);
  const plotH = Math.max(1, cssH - padY * 2);

  const xOf = (t: number): number =>
    padX + ((t - bounds.tMin) / Math.max(1e-6, bounds.tMax - bounds.tMin)) * plotW;
  const yOf = (c: number): number =>
    padY + plotH - ((c - bounds.cMin) / Math.max(1e-6, bounds.cMax - bounds.cMin)) * plotH;

  // Zero (median) reference line.
  const zeroY = yOf(0);
  ctx.strokeStyle = COLORS.zero;
  ctx.lineWidth = 1;
  ctx.setLineDash([3, 3]);
  ctx.beginPath();
  ctx.moveTo(padX, zeroY);
  ctx.lineTo(padX + plotW, zeroY);
  ctx.stroke();
  ctx.setLineDash([]);

  // Polyline per voiced segment. `ctx.beginPath()` is restarted on each
  // segment so unvoiced runs render as gaps.
  ctx.strokeStyle = COLORS.voiced;
  ctx.lineWidth = 2;
  ctx.lineJoin = "round";
  ctx.lineCap = "round";
  for (const seg of segments) {
    if (seg.frames.length === 0) continue;
    ctx.beginPath();
    let started = false;
    for (const f of seg.frames) {
      const x = xOf(f.tMs);
      const y = yOf(f.centsFromMedian);
      if (!started) {
        ctx.moveTo(x, y);
        started = true;
      } else {
        ctx.lineTo(x, y);
      }
    }
    ctx.stroke();
  }

  // Faint fill under the trace to add a redundant, non-color shape cue.
  ctx.fillStyle = COLORS.voicedFill;
  for (const seg of segments) {
    if (seg.frames.length < 2) continue;
    const firstFrame = seg.frames[0];
    const lastFrame = seg.frames[seg.frames.length - 1];
    if (firstFrame === undefined || lastFrame === undefined) continue;
    ctx.beginPath();
    ctx.moveTo(xOf(firstFrame.tMs), zeroY);
    for (const f of seg.frames) {
      ctx.lineTo(xOf(f.tMs), yOf(f.centsFromMedian));
    }
    ctx.lineTo(xOf(lastFrame.tMs), zeroY);
    ctx.closePath();
    ctx.fill();
  }
}

function buildAriaLabel(
  summary: AnalysisSummary | undefined,
  contour: ContourResult | undefined,
): string {
  if (summary === undefined && contour === undefined) {
    return "Pitch contour: no analysis data yet";
  }
  const median = summary ?? contour;
  if (median === undefined) return "Pitch contour";
  // Compute the visible range from the contour if available.
  let rangeCents = 0;
  if (contour !== undefined && contour.frames.length > 0) {
    let lo = Number.POSITIVE_INFINITY;
    let hi = Number.NEGATIVE_INFINITY;
    for (const f of contour.frames) {
      if (!f.voiced) continue;
      if (f.centsFromMedian < lo) lo = f.centsFromMedian;
      if (f.centsFromMedian > hi) hi = f.centsFromMedian;
    }
    if (Number.isFinite(lo) && Number.isFinite(hi)) {
      rangeCents = Math.round(hi - lo);
    }
  }
  const voicedPct = Math.round(median.voicedRatio * 100);
  return `Pitch contour: median MIDI ${median.medianMidi}, range ${rangeCents} cents, ${voicedPct}% voiced`;
}

export function ContourLine({ summary, contour }: ContourLineProps): ReactNode {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  // Stash the downsampled segments in a ref so the rAF redraw and any
  // ResizeObserver / DPR changes can repaint without re-running LTTB.
  const segmentsRef = useRef<DrawableSegment[]>([]);
  const boundsRef = useRef<PaintBounds>({
    tMin: 0,
    tMax: 1,
    cMin: -DEFAULT_RANGE,
    cMax: DEFAULT_RANGE,
  });

  const ariaLabel = useMemo(() => buildAriaLabel(summary, contour), [summary, contour]);

  // Recompute the downsampled segments + bounds when the contour changes.
  useEffect(() => {
    if (contour === undefined || contour.frames.length === 0) {
      segmentsRef.current = [];
      boundsRef.current = { tMin: 0, tMax: 1, cMin: -DEFAULT_RANGE, cMax: DEFAULT_RANGE };
      return;
    }
    const lastFrame = contour.frames[contour.frames.length - 1];
    const firstFrame = contour.frames[0];
    const durationMs =
      lastFrame !== undefined && firstFrame !== undefined ? lastFrame.tMs - firstFrame.tMs : 0;
    const ds = downsample(contour.frames, durationMs);
    segmentsRef.current = toSegments(ds);
    boundsRef.current = computeBounds(ds);
  }, [contour]);

  // Canvas wiring: DPR scaling, resize observer, matchMedia listener, and
  // a one-shot rAF redraw on every paint trigger.
  useEffect(() => {
    const canvas = canvasRef.current;
    if (canvas === null) return undefined;
    const ctx = canvas.getContext("2d");
    if (ctx === null) return undefined;

    const repaint = (): void => {
      paint(ctx, canvas.clientWidth, canvas.clientHeight, segmentsRef.current, boundsRef.current);
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

    // Reduced-motion: paint synchronously after layout (already done
    // above). A motion-permitted environment uses a single rAF tick so
    // the paint lands on the next animation frame; the contour is
    // static so the rAF loop exits after one paint per data update.
    const motionMql = window.matchMedia("(prefers-reduced-motion: reduce)");
    let raf = 0;
    if (!motionMql.matches) {
      raf = window.requestAnimationFrame(() => {
        repaint();
      });
    }

    return () => {
      if (raf !== 0) window.cancelAnimationFrame(raf);
      ro.disconnect();
      if (mql !== null && mqlListener !== null) mql.removeEventListener("change", mqlListener);
    };
  }, [contour, summary]);

  return (
    <figure
      data-testid="contour-figure"
      role="img"
      aria-label={ariaLabel}
      className="m-0 flex flex-col gap-1"
    >
      <canvas
        ref={canvasRef}
        aria-hidden="true"
        data-testid="contour-canvas"
        className="block h-32 w-full rounded-md bg-slate-900"
      />
      <figcaption className="sr-only">{ariaLabel}</figcaption>
    </figure>
  );
}
