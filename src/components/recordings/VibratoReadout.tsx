// VibratoReadout — vibrato readout.
//
// Mounted to the right of RangeReadout in a 2-column grid (sibling of
// AnalysisSummary; below the summary card and above ContourLine). Pure-
// presentational: the parent owns the single Zustand subscription and
// forwards `summary` so this component stays equality-stable.
//
// Empty state: `summary?.vibrato === undefined || vibratoRatio < 0.05`
//   → render the single empty paragraph and skip the meter + dots.
//
// The rate bar is an SVG `role="meter"` carrying:
//   - `aria-valuemin={0}`, `aria-valuemax={10}`, `aria-valuenow={rate}`
//   - `aria-valuetext` composed as e.g. "5.4 Hz, typical voice range"
//     ("typical voice range" when 4..7 Hz, "below" / "above" otherwise).
// The 4–7 Hz typical band is a fixed `<rect data-testid="typical-band">`
// at x=40%/width=30%; the indicator is a static `<line>`.
//
// Reduced-motion contract:
//   The indicator's x-position is driven by the SVG `x1` / `x2`
//   geometric attributes, not a CSS-animatable property — Tailwind
//   `transition-all` therefore does NOT animate the indicator's position
//   even on the default-motion branch. The `transition-none` /
//   `transition-all` class swap is preserved as a discoverable signal
//   for AT inspectors and the e2e suite (Playwright asserts that the
//   `transition-none` class is present when `prefers-reduced-motion:
//   reduce` matches), but it is vacuously satisfied: there is no
//   animation in either branch. If a future revision drives the
//   indicator via a `transform: translateX(...)` on a wrapping `<g>`,
//   the class swap becomes load-bearing and this comment must be
//   updated alongside the geometry change.
//
// Per-window dot strip below the bar — one `<span data-testid="vibrato-
// window-dot">` per `windows[i]`. Color is derived from `confidence`:
//   < 0.5  → slate (low)
//   < 0.85 → cyan  (mid)
//   else   → emerald (high)
// The strip is informational; not focusable.
//
//   src/components/CentsMeter.tsx (canonical reduced-motion pattern)

import { useEffect, useState, type ReactNode } from "react";
import type { AnalysisSummary } from "@/types/analysis";

export interface VibratoReadoutProps {
  summary: AnalysisSummary | undefined;
}

const RATE_SCALE_MAX = 10; // Hz — meter ceiling
const TYPICAL_LOW = 4; // Hz
const TYPICAL_HIGH = 7; // Hz
/** Minimum `vibrato_ratio` for the readout to leave the empty state.
 *  The Rust analyzer's `vibrato_no_vibrato_returns_low_ratio` test asserts
 *  `vibrato_ratio < 0.05` for clean no-vibrato fixtures (strict `<`); the
 *  readout uses the same strict `<` shape so a take whose ratio sits
 *  exactly at the boundary (0.05) falls into the readout's happy path —
 *  by design, mirroring the Rust threshold's openness on the floor side. */
const EMPTY_RATIO_THRESHOLD = 0.05;

/** SSR-safe matchMedia hook. Mirrors the CentsMeter pattern: start with
 *  the synchronous match value and re-subscribe on changes so a runtime
 *  toggle of the OS-level preference is honoured without a remount. */
function useMatchesReduceMotion(): boolean {
  const [matches, setMatches] = useState<boolean>(() => {
    if (typeof window === "undefined") return false;
    return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
  });
  useEffect(() => {
    if (typeof window === "undefined") return undefined;
    const mql = window.matchMedia("(prefers-reduced-motion: reduce)");
    const onChange = (): void => setMatches(mql.matches);
    mql.addEventListener("change", onChange);
    return (): void => {
      mql.removeEventListener("change", onChange);
    };
  }, []);
  return matches;
}

function buildAriaText(rateHz: number): string {
  const rounded = rateHz.toFixed(1);
  if (rateHz < TYPICAL_LOW) return `${rounded} Hz, below typical range`;
  if (rateHz > TYPICAL_HIGH) return `${rounded} Hz, above typical range`;
  return `${rounded} Hz, typical voice range`;
}

function dotColorClass(confidence: number): string {
  if (confidence < 0.5) return "bg-slate-500";
  if (confidence < 0.85) return "bg-cyan-400";
  return "bg-emerald-400";
}

export function VibratoReadout({ summary }: VibratoReadoutProps): ReactNode {
  const reducedMotion = useMatchesReduceMotion();
  const vibrato = summary?.vibrato;
  const isEmpty = vibrato === undefined || vibrato.vibratoRatio < EMPTY_RATIO_THRESHOLD;

  if (isEmpty) {
    return (
      <section
        role="group"
        aria-label="Vibrato analysis"
        data-testid="vibrato-readout"
        className="flex flex-col gap-2 rounded-md border border-slate-700 bg-slate-900/40 p-3"
      >
        <h3 className="text-xs font-semibold uppercase tracking-wide text-slate-300">Vibrato</h3>
        <p data-testid="vibrato-empty" className="text-xs text-slate-400">
          No vibrato detected.
        </p>
      </section>
    );
  }

  // `vibrato` is non-undefined here because `isEmpty` short-circuited on
  // both `=== undefined` and the ratio threshold — TypeScript's narrowing
  // already follows the second branch. Locally rebind so the templates
  // below read cleanly.
  const v = vibrato;
  const rateHz = v.medianRateHz;
  const rateClamped = Math.max(0, Math.min(RATE_SCALE_MAX, rateHz));
  const ratePct = (rateClamped / RATE_SCALE_MAX) * 100;
  // aria-valuenow ships as a number on the JSX side; React serialises
  // numeric values via toString(), but `5.4` round-trips directly.
  // Specs assert against the rendered attribute string ("5.4").
  const ariaValueNow = Number(rateHz.toFixed(1));
  const ariaValueText = buildAriaText(rateHz);
  const extentLabel = `${Math.round(v.medianExtentCents)} ¢`;
  const ratioPct = `${Math.round(v.vibratoRatio * 100)}%`;

  // Reduced-motion → suppress the transform/width transition on the
  // indicator. Visual-only feedback honours prefers-reduced-
  // motion. The class string is explicit so Playwright can match
  // `[class*="transition-none"]`. The class is applied both to the
  // indicator <line> (for the visual transition suppression) and to the
  // wrapping role="meter" div (so the Playwright `toBeVisible()` query —
  // which honours aria-hidden ancestors of the SVG — can hit a
  // non-aria-hidden element carrying the same class).
  const indicatorClass = [
    "fill-cyan-400 stroke-cyan-400",
    reducedMotion ? "transition-none" : "transition-all duration-200 ease-out",
  ].join(" ");
  const meterMotionClass = reducedMotion ? "transition-none" : "transition-all duration-200";

  return (
    <section
      role="group"
      aria-label="Vibrato analysis"
      data-testid="vibrato-readout"
      className="flex flex-col gap-2 rounded-md border border-slate-700 bg-slate-900/40 p-3"
    >
      <h3 className="text-xs font-semibold uppercase tracking-wide text-slate-300">Vibrato</h3>

      <div className="flex flex-wrap items-end gap-x-4 gap-y-1 text-sm">
        <div className="flex flex-col">
          <span className="text-xs font-semibold uppercase tracking-wide text-slate-400">Rate</span>
          <span className="font-mono text-slate-100" data-testid="vibrato-rate">
            {`${rateHz.toFixed(1)} Hz`}
          </span>
        </div>
        <div className="flex flex-col">
          <span className="text-xs font-semibold uppercase tracking-wide text-slate-400">
            Extent
          </span>
          <span className="font-mono text-slate-100" data-testid="vibrato-extent">
            {extentLabel}
          </span>
        </div>
        <div className="flex flex-col">
          <span className="text-xs font-semibold uppercase tracking-wide text-slate-400">
            Ratio
          </span>
          <span className="font-mono text-slate-100" data-testid="vibrato-ratio">
            {ratioPct}
          </span>
        </div>
      </div>

      {/* Rate bar — SVG with role="meter" on the wrapping div so AT users
          read the value via aria-valuenow / aria-valuetext. The
          `<rect data-testid="typical-band">` paints the 4–7 Hz typical-rate band
          under the indicator; the indicator itself is a static <line>. */}
      <div
        role="meter"
        aria-label="Vibrato rate"
        aria-valuemin={0}
        aria-valuemax={RATE_SCALE_MAX}
        aria-valuenow={ariaValueNow}
        aria-valuetext={ariaValueText}
        data-testid="vibrato-meter"
        className={["w-full", meterMotionClass].join(" ")}
      >
        <svg
          aria-hidden="true"
          viewBox="0 0 100 12"
          preserveAspectRatio="none"
          className="block h-3 w-full rounded-md bg-slate-800"
        >
          {/* Track is the SVG bg via the parent rounded-md / bg-slate-800.
              Typical-band rect: x=40%, width=30% (i.e. 4..7 Hz on a 0..10
              scale). */}
          <rect
            data-testid="typical-band"
            x={(TYPICAL_LOW / RATE_SCALE_MAX) * 100}
            y={0}
            width={((TYPICAL_HIGH - TYPICAL_LOW) / RATE_SCALE_MAX) * 100}
            height={12}
            className="fill-cyan-900/60"
          />
          {/* Indicator line — static <line>; reduced-motion suppresses
              the transition class so position changes do not animate. */}
          <line
            data-testid="vibrato-indicator"
            x1={ratePct}
            x2={ratePct}
            y1={0}
            y2={12}
            strokeWidth={2}
            className={indicatorClass}
          />
        </svg>
      </div>

      {/* Per-window dot strip — informational, not focusable. */}
      {v.windows.length > 0 ? (
        <div
          aria-hidden="true"
          data-testid="vibrato-window-strip"
          className="flex flex-wrap items-center gap-1"
        >
          {v.windows.map((w, idx) => (
            <span
              key={`${w.tMs}-${idx}`}
              data-testid="vibrato-window-dot"
              className={["inline-block h-2 w-2 rounded-full", dotColorClass(w.confidence)].join(
                " ",
              )}
            />
          ))}
        </div>
      ) : null}
    </section>
  );
}
