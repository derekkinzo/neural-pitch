// CentsMeter — canvas-driven -50/+50 ¢ deviation bar.
//
// Reads the rAF ring directly. The component renders a `role="meter"` wrapper
// around a `<canvas>`. The canvas itself is `aria-hidden="true"` because the
// semantic meter meaning lives on the wrapping div; ATs that would otherwise
// announce empty `<canvas>` content are silenced.
//
// ARIA write cadence: aria-valuenow / aria-valuetext are throttled to at
// most one update every `ARIA_THROTTLE_MS` so AT screen-reader engines
// (NVDA / JAWS / VoiceOver) do not re-poll the live region at the canvas
// paint rate. Canvas paint stays at the rAF cadence (or the
// reduced-motion cadence below).
//
// Motion encoding (WCAG 1.4.1 — color is not the sole channel):
//   - In-tune (-5..+5 ¢)  → filled-triangle needle (shape change)
//   - Sharp (>+5 ¢)        → vertical line, *right* of center (position)
//   - Flat  (<-5 ¢)        → vertical line, *left*  of center (position)
//   The sharp/flat color difference (amber vs orange) is a *redundant*
//   channel; the primary distinction is needle x-position relative to the
//   center reference line.
//
// `prefers-reduced-motion: reduce` downshifts the rAF cadence to
// `REDUCED_PAINT_MS` so the needle does not animate at 60 Hz; ARIA updates
// are still throttled to `ARIA_THROTTLE_MS` so AT users get a stable,
// debounced readout.
//
// DPR (devicePixelRatio) changes — the user dragging the window between
// monitors of different DPI, OS-level UI zoom, or browser ctrl-+ zoom — do
// not always fire `ResizeObserver`. We additionally subscribe to a
// `matchMedia("(resolution: <Ndppx>)")` listener and re-scale on every
// transition.
//

import { useEffect, useRef, type ReactNode } from "react";
import { SILENT_PITCH, type PitchUpdate } from "@/types/pitch";
import { scaleForDpr } from "@/lib/canvas-dpr";
import { hzToNote, formatNoteShort } from "@/lib/note-format";
import { useSettingsStore } from "@/stores/settingsStore";
import type { RingBuffer } from "@/lib/ring";

export interface CentsMeterProps {
  ringRef: React.RefObject<RingBuffer<PitchUpdate>>;
}

const RANGE = 50; // ± cents
const ARIA_THROTTLE_MS = 250;
const REDUCED_PAINT_MS = 250;

const COLORS = {
  track: "#1e293b", // slate-800
  band: "#0e7490", // cyan-700
  tick: "#475569", // slate-600
  axis: "#22d3ee", // cyan-400
  inTune: "#22d3ee",
  sharp: "#fbbf24", // amber-400
  flat: "#fb923c", // orange-400
} as const;

function paint(
  ctx: CanvasRenderingContext2D,
  cssW: number,
  cssH: number,
  cents: number,
  voiced: boolean,
): void {
  ctx.clearRect(0, 0, cssW, cssH);

  const cx = cssW / 2;
  const baselineY = cssH / 2;

  // Track.
  ctx.fillStyle = COLORS.track;
  ctx.fillRect(0, baselineY - 4, cssW, 8);

  // In-tune band (±5 ¢).
  const bandHalf = ((5 / RANGE) * cssW) / 2;
  ctx.fillStyle = COLORS.band;
  ctx.fillRect(cx - bandHalf, baselineY - 6, bandHalf * 2, 12);

  // Tick marks every 10 cents.
  ctx.strokeStyle = COLORS.tick;
  ctx.lineWidth = 1;
  ctx.beginPath();
  for (let c = -RANGE; c <= RANGE; c += 10) {
    const x = cx + (c / RANGE) * (cssW / 2);
    const major = c % 50 === 0;
    const tickH = major ? 14 : 8;
    ctx.moveTo(x, baselineY - tickH);
    ctx.lineTo(x, baselineY + tickH);
  }
  ctx.stroke();

  // Center reference (0¢).
  ctx.strokeStyle = COLORS.axis;
  ctx.lineWidth = 2;
  ctx.beginPath();
  ctx.moveTo(cx, 6);
  ctx.lineTo(cx, cssH - 6);
  ctx.stroke();

  if (!voiced) return;

  // Needle — line normally, filled triangle when in-tune.
  const clamped = Math.max(-RANGE, Math.min(RANGE, cents));
  const x = cx + (clamped / RANGE) * (cssW / 2);
  const inTune = Math.abs(cents) <= 5;
  if (inTune) {
    ctx.fillStyle = COLORS.inTune;
    ctx.beginPath();
    ctx.moveTo(x, baselineY - 28);
    ctx.lineTo(x - 12, baselineY + 18);
    ctx.lineTo(x + 12, baselineY + 18);
    ctx.closePath();
    ctx.fill();
  } else {
    const tooSharp = cents > 0;
    ctx.strokeStyle = tooSharp ? COLORS.sharp : COLORS.flat;
    ctx.lineWidth = 4;
    ctx.beginPath();
    ctx.moveTo(x, 8);
    ctx.lineTo(x, cssH - 8);
    ctx.stroke();
  }
}

function buildAriaText(cents: number, voiced: boolean, noteLabel: string): string {
  if (!voiced) return "no signal";
  const direction = Math.abs(cents) <= 5 ? "in tune" : cents > 0 ? "sharp" : "flat";
  const sign = cents > 0 ? "+" : "";
  return `${noteLabel} ${sign}${cents.toFixed(0)} cents (${direction})`;
}

function classifyState(u: PitchUpdate): "silent" | "in-tune" | "sharp" | "flat" {
  if (!u.voiced) return "silent";
  if (Math.abs(u.smoothed_cents) <= 5) return "in-tune";
  return u.smoothed_cents > 0 ? "sharp" : "flat";
}

export function CentsMeter({ ringRef }: CentsMeterProps): ReactNode {
  const rootRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);

  useEffect(() => {
    const canvas = canvasRef.current;
    const root = rootRef.current;
    if (canvas === null || root === null) return undefined;
    const ctx = canvas.getContext("2d");
    if (ctx === null) return undefined;

    scaleForDpr(canvas, ctx);

    // ResizeObserver: CSS-size changes trigger a re-scale.
    const ro = new ResizeObserver(() => {
      if (canvasRef.current !== null) scaleForDpr(canvasRef.current, ctx);
    });
    ro.observe(canvas);

    // matchMedia: subscribe to *the current* devicePixelRatio. When the
    // user moves the window across monitors, OS-level zooms, or browser-
    // zooms, the listener fires; we re-create the listener keyed on the
    // new DPR so the next change still wakes us. The ResizeObserver path
    // does NOT cover this case.
    let mql: MediaQueryList | null = null;
    let mqlListener: ((e: MediaQueryListEvent) => void) | null = null;
    const subscribeDpr = (): void => {
      if (mql !== null && mqlListener !== null) {
        mql.removeEventListener("change", mqlListener);
      }
      const dpr = window.devicePixelRatio || 1;
      mql = window.matchMedia(`(resolution: ${dpr}dppx)`);
      mqlListener = () => {
        if (canvasRef.current !== null) scaleForDpr(canvasRef.current, ctx);
        subscribeDpr();
      };
      mql.addEventListener("change", mqlListener);
    };
    subscribeDpr();

    // Reduced-motion cadence. The cadence is locked in at mount: a
    // runtime flip of the OS-level setting requires a remount of the
    // CentsMeter (e.g. navigating away from / back to the tuner) to
    // pick up the new branch. This matches the rest of the meter's
    // mount-once contract.
    const motionMql = window.matchMedia("(prefers-reduced-motion: reduce)");
    const reducedMotion = motionMql.matches;

    let raf = 0;
    let intervalId: number | null = null;
    let lastState = "";
    let lastAriaCents = Number.NaN;
    let lastNoteLabel = "";
    let lastAriaWriteAt = 0;

    const renderOnce = (): void => {
      const ring = ringRef.current;
      const u = ring?.peekLatest() ?? SILENT_PITCH;
      paint(ctx, canvas.clientWidth, canvas.clientHeight, u.smoothed_cents, u.voiced);
      const state = classifyState(u);
      const stateChanged = state !== lastState;
      if (stateChanged) {
        lastState = state;
        root.setAttribute("data-state", state);
      }
      // ARIA — throttle writes so AT engines don't re-poll the live region
      // at 60 Hz. Always write on a state transition (even mid-throttle)
      // so silence/in-tune/sharp/flat never lag.
      const a4 = useSettingsStore.getState().a4Hz;
      const note = hzToNote(u.voiced ? u.f0_hz : 0, a4);
      const noteLabel = u.voiced ? formatNoteShort(note) : "—";
      const centsRounded = Math.round(u.smoothed_cents);
      const now = performance.now();
      const valueChanged = centsRounded !== lastAriaCents || noteLabel !== lastNoteLabel;
      const throttleElapsed = now - lastAriaWriteAt >= ARIA_THROTTLE_MS;
      if ((stateChanged || valueChanged) && (stateChanged || throttleElapsed)) {
        lastAriaCents = centsRounded;
        lastNoteLabel = noteLabel;
        lastAriaWriteAt = now;
        root.setAttribute("aria-valuenow", String(centsRounded));
        root.setAttribute("aria-valuetext", buildAriaText(u.smoothed_cents, u.voiced, noteLabel));
      }
    };

    const startRaf = (): void => {
      const tick = (): void => {
        renderOnce();
        raf = window.requestAnimationFrame(tick);
      };
      raf = window.requestAnimationFrame(tick);
    };
    const startInterval = (): void => {
      intervalId = window.setInterval(renderOnce, REDUCED_PAINT_MS);
      renderOnce();
    };

    // Choose cadence at mount; runtime flips of the OS reduced-motion
    // setting need a remount to take effect (see the matchMedia comment
    // above for the contract).
    if (reducedMotion) startInterval();
    else startRaf();

    return () => {
      if (raf !== 0) window.cancelAnimationFrame(raf);
      if (intervalId !== null) window.clearInterval(intervalId);
      ro.disconnect();
      if (mql !== null && mqlListener !== null) mql.removeEventListener("change", mqlListener);
    };
  }, [ringRef]);

  return (
    <div
      ref={rootRef}
      role="meter"
      aria-label="Pitch deviation in cents"
      aria-valuemin={-RANGE}
      aria-valuemax={RANGE}
      aria-valuenow={0}
      aria-valuetext="no signal"
      data-state="silent"
      data-testid="cents-meter"
      className="w-full"
    >
      <canvas
        ref={canvasRef}
        aria-hidden="true"
        data-testid="cents-meter-canvas"
        className="block h-24 w-full rounded-md bg-slate-900"
      />
    </div>
  );
}
