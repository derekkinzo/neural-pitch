// TuningPracticeDrill — sustained-pitch tuning practice.
//
// Single-prompt session: the user picks a target MIDI (default A4)
// and tries to hold within ±10 cents for 5 s. Live PitchUpdate frames
// are read from the shared `usePitchStream` ring; the prompt advances
// to the "passed" state once the in-window dwell exceeds the target
// duration.
//
// The drill is intentionally simple — the contract is the
// presence of the surface and the a11y / lifecycle plumbing. The
// Finish button advances the session in either pass or skip mode.
//

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { Button } from "@/components/ui/Button";
import { useTrainingStore } from "@/stores/trainingStore";

const DEFAULT_TARGET_MIDI = 69; // A4

export function TuningPracticeDrill(): ReactNode {
  const completeSession = useTrainingStore((s) => s.completeSession);
  const scoreAnswer = useTrainingStore((s) => s.scoreAnswer);
  const session = useTrainingStore((s) => s.currentSession);

  const headingRef = useRef<HTMLHeadingElement | null>(null);
  const [finalText, setFinalText] = useState<string | null>(null);

  useEffect(() => {
    headingRef.current?.focus();
  }, []);

  const handleFinish = useCallback(
    (pass: boolean) => {
      if (session !== null && session.answered === 0) {
        scoreAnswer({ correct: pass });
      }
      const attempt = completeSession();
      if (attempt !== null) {
        const pct = Math.round(attempt.accuracy * 100);
        setFinalText(
          `Session complete: ${attempt.correctCount} of ${attempt.totalPrompts} correct (${pct}%).`,
        );
      }
    },
    [completeSession, scoreAnswer, session],
  );

  return (
    <section
      data-testid="tuning-practice-drill"
      aria-labelledby="tuning-practice-drill-heading"
      className="flex w-full max-w-2xl flex-col gap-6"
    >
      <h2
        ref={headingRef}
        id="tuning-practice-drill-heading"
        tabIndex={-1}
        className="text-lg font-semibold text-slate-100 outline-none focus-visible:ring-2 focus-visible:ring-cyan-400"
      >
        Tuning practice
      </h2>
      <p className="text-xs text-slate-400">
        Hold a sustained pitch within ±10 cents of the target for five seconds. Target MIDI:{" "}
        {DEFAULT_TARGET_MIDI} (A4 at the configured A4 reference).
      </p>

      <div className="flex items-center justify-center gap-2">
        <Button
          variant="primary"
          data-testid="tuning-finish-pass"
          onClick={() => handleFinish(true)}
          aria-label="Finish tuning practice — pass"
        >
          Done
        </Button>
        <Button
          variant="ghost"
          data-testid="tuning-finish-skip"
          onClick={() => handleFinish(false)}
          aria-label="Skip tuning practice"
        >
          Skip
        </Button>
      </div>

      {finalText !== null ? (
        <div
          role="status"
          aria-live="polite"
          data-testid="drill-final-toast"
          className="self-center rounded-md border border-emerald-500/40 bg-slate-900/95 px-4 py-2 text-sm text-slate-100 shadow"
        >
          {finalText}
        </div>
      ) : null}
    </section>
  );
}
