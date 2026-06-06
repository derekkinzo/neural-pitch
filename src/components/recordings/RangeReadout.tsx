// RangeReadout — Phase 2.3 vocal-range readout.
//
// Mounted inside RecordingDetail as a sibling of AnalysisSummary, *below*
// the summary card and above ContourLine, in a 2-column grid alongside
// VibratoReadout. Pure-presentational: the parent owns the single Zustand
// subscription and forwards `summary` so this component stays equality-
// stable across unrelated store mutations.
//
// Empty / insufficient state:
//   `summary?.range === undefined` OR `voicedFrameCount < 250` (≈ 5 s of
//   voiced audio at the 50 ms hop) — render the single empty paragraph and
//   skip the numeric pair + voice-type hint pills.
//
// Voice-type hint framing (a11y):
//   The hint pills are rendered as overlap claims, not identity verdicts —
//   each pill reads "<Type> range" and a visible framing label
//   ("Range overlaps:") sits inline with the pill row so a sighted user
//   never sees a bare verdict-shaped label. A short visible disclaimer
//   line directly below the pill row covers both sighted and AT users in
//   one shot — keyboard-only sighted users do not depend on a `title`
//   tooltip surfacing on focus (Chromium does not render `title` on
//   keyboard focus). The disclaimer paragraph is associated to the pill
//   row via `aria-describedby` so AT engines announce it exactly once per
//   focus event rather than twice (once from `aria-label` + once from a
//   sibling sr-only span).
//
// Cross-references:
//   docs/design/DESIGN.md §7.5 (Phase 2.3 frontend additions)
//   docs/adr/0006-visual-only-feedback-prefers-reduced-motion.md
//   src/components/recordings/RecordingDetail.tsx (parent — single subscription)

import type { ReactNode } from "react";
import type { AnalysisSummary } from "@/types/analysis";
import { formatMidiNote } from "@/lib/note-format";

export interface RangeReadoutProps {
  summary: AnalysisSummary | undefined;
  a4Hz: number;
}

/** ~250 frames at the 50 ms hop ≈ 5 s of voiced audio. Below this we
 *  cannot meaningfully estimate a 5th/95th percentile. */
const VOICED_FRAME_THRESHOLD = 250;

const VOICE_TYPE_DISCLAIMER =
  "Your comfortable range overlaps these voice types per New Grove vocal-range conventions. This is not a vocal coach assessment.";

/** Stable id for the visible disclaimer paragraph. The pill row references
 *  it via `aria-describedby` so AT engines announce the disclaimer exactly
 *  once when focus enters the group. */
const VOICE_HINT_DISCLAIMER_ID = "range-readout-voice-hint-disclaimer";

export function RangeReadout({ summary, a4Hz }: RangeReadoutProps): ReactNode {
  const range = summary?.range;
  const insufficient = range === undefined || range.voicedFrameCount < VOICED_FRAME_THRESHOLD;

  return (
    <section
      role="group"
      aria-label="Vocal range report"
      data-testid="range-readout"
      className="flex flex-col gap-2 rounded-md border border-slate-700 bg-slate-900/40 p-3"
    >
      <h3 className="text-xs font-semibold uppercase tracking-wide text-slate-300">Vocal range</h3>

      {insufficient ? (
        <p data-testid="range-empty" className="text-xs text-slate-400">
          Not enough voiced material to compute a range. Sing at least 5 seconds of voiced audio.
        </p>
      ) : (
        <>
          <div className="flex flex-col gap-1">
            <span className="text-xs font-semibold uppercase tracking-wide text-slate-400">
              Comfortable
            </span>
            <span className="text-base font-medium text-slate-100" data-testid="range-comfortable">
              {`${formatMidiNote(range.comfortableLowMidi, a4Hz)} - ${formatMidiNote(
                range.comfortableHighMidi,
                a4Hz,
              )}`}
            </span>
          </div>

          <div className="flex flex-col gap-1">
            <span className="text-xs font-semibold uppercase tracking-wide text-slate-400">
              Full range
            </span>
            <span className="text-sm text-slate-200" data-testid="range-full">
              {`${formatMidiNote(range.fullLowMidi, a4Hz)} - ${formatMidiNote(
                range.fullHighMidi,
                a4Hz,
              )}`}
            </span>
          </div>

          <span className="text-xs text-slate-400" data-testid="range-voiced-frames">
            {`${range.voicedFrameCount} voiced frames`}
          </span>

          {range.voiceTypeHints.length > 0 ? (
            <div className="flex flex-col gap-1">
              <div
                className="flex flex-wrap items-center gap-1"
                aria-describedby={VOICE_HINT_DISCLAIMER_ID}
              >
                {/* Visible framing label so sighted users do not read the
                    pills as identity verdicts ("You are an Alto") — pills
                    follow as "<Type> range" overlap claims. */}
                <span className="text-xs text-slate-400" data-testid="voice-hint-framing">
                  Range overlaps:
                </span>
                {range.voiceTypeHints.map((hint, idx) => (
                  <span
                    // Hint labels are short and stable across renders; we use
                    // `${hint}-${idx}` to disambiguate duplicate labels per
                    // React's reconciler key contract without paying for a
                    // separate id field on the wire format.
                    key={`${hint}-${idx}`}
                    data-testid="voice-hint-pill"
                    className="rounded-full border border-slate-600 px-2 py-0.5 text-xs text-slate-200"
                  >
                    {`${hint} range`}
                  </span>
                ))}
              </div>
              {/* Visible disclaimer line — covers sighted + keyboard +
                  AT users in one shot. The `aria-describedby` reference
                  above means AT engines announce this exactly once when
                  focus enters the pill row. */}
              <p
                id={VOICE_HINT_DISCLAIMER_ID}
                className="text-xs text-slate-400"
                data-testid="voice-hint-disclaimer"
              >
                {VOICE_TYPE_DISCLAIMER}
              </p>
            </div>
          ) : null}
        </>
      )}
    </section>
  );
}
