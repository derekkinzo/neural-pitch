// SightSingingDrill — Phase 4 sight-singing flow.
//
// Mounts the karaoke ribbon over an active melody (loaded from the
// training mock or a future built-in catalogue), arms the
// `match-update` channel listener via `useDrillMatchStream`, and
// exposes a "Finish" affordance so the user can complete the session
// from the keyboard or mouse alike.
//
// The drill mounts a single prompt (the melody) so the session is
// `totalPrompts = 1`; finishing the drill calls
// `completeSession()` on the training store and surfaces the same
// final-score toast the other drills use.
//

import { useCallback, useEffect, useRef, useState, type ReactNode } from "react";
import { Button } from "@/components/ui/Button";
import { KaraokeRibbon } from "@/components/training/KaraokeRibbon";
import { useDrillMatchStream } from "@/hooks/useDrillMatchStream";
import { useSettingsStore } from "@/stores/settingsStore";
import { useTrainingStore } from "@/stores/trainingStore";
import type { Melody } from "@/types/training";

const FALLBACK_MELODY: Melody = {
  id: "melody-c-major-octave",
  tonicMidi: 60,
  notes: [0, 2, 4, 5, 7, 9, 11, 12].map((step, i) => ({
    midi: 60 + step,
    startMs: i * 250,
    durationMs: 250,
  })),
};

interface TrainingHooks {
  training?: { melody?: Melody };
}

function loadMelodyFromHooks(): Melody {
  if (typeof window === "undefined") return FALLBACK_MELODY;
  const w = window as Window & { __neuralPitchTestHooks?: TrainingHooks };
  return w.__neuralPitchTestHooks?.training?.melody ?? FALLBACK_MELODY;
}

export function SightSingingDrill(): ReactNode {
  const a4Hz = useSettingsStore((s) => s.a4Hz);
  const completeSession = useTrainingStore((s) => s.completeSession);
  const scoreAnswer = useTrainingStore((s) => s.scoreAnswer);
  const setActiveMelody = useTrainingStore((s) => s.setActiveMelody);
  const activeMelody = useTrainingStore((s) => s.activeMelody);
  const session = useTrainingStore((s) => s.currentSession);

  const headingRef = useRef<HTMLHeadingElement | null>(null);
  const [finalText, setFinalText] = useState<string | null>(null);

  // Wire the match-update channel listener so the karaoke ribbon
  // receives MatchUpdate frames. Tear-down tolerates the receiver
  // closing early (the helper itself is no-op when nothing is mounted).
  useDrillMatchStream(session !== null);

  // Load the active melody once the drill mounts. Effect depends only
  // on `setActiveMelody` (a stable Zustand setter) so re-renders do
  // not churn the slot.
  useEffect(() => {
    setActiveMelody(loadMelodyFromHooks());
    return () => {
      setActiveMelody(null);
    };
  }, [setActiveMelody]);

  // Move keyboard focus to the heading on mount so AT users land on
  // the drill title instead of an unrelated browser default. Heading
  // carries `tabIndex={-1}` so it is programmatically focusable
  // without becoming a tab stop.
  useEffect(() => {
    headingRef.current?.focus();
  }, []);

  const handleFinish = useCallback(() => {
    // Sight-singing is a single-prompt session; mark the prompt
    // answered as "correct" by default — a future scorer reduction
    // will replace this with a real verdict from `liveMatch`.
    if (session !== null && session.answered === 0) {
      scoreAnswer({ correct: true });
    }
    const attempt = completeSession();
    if (attempt !== null) {
      const pct = Math.round(attempt.accuracy * 100);
      setFinalText(
        `Session complete: ${attempt.correctCount} of ${attempt.totalPrompts} correct (${pct}%).`,
      );
    }
  }, [completeSession, scoreAnswer, session]);

  const melody = activeMelody ?? FALLBACK_MELODY;

  return (
    <section
      data-testid="sight-singing-drill"
      aria-labelledby="sight-singing-drill-heading"
      className="flex w-full max-w-3xl flex-col gap-6"
    >
      <h2
        ref={headingRef}
        id="sight-singing-drill-heading"
        tabIndex={-1}
        className="text-lg font-semibold text-slate-100 outline-none focus-visible:ring-2 focus-visible:ring-cyan-400"
      >
        Sight-singing
      </h2>
      <p className="text-xs text-slate-400">
        Sing along — bars light up cyan when you are in tune. The ribbon scrolls in sync with the
        target melody (a static snapshot if you have reduced-motion enabled).
      </p>

      <KaraokeRibbon melody={melody} a4Hz={a4Hz} />

      <div className="flex items-center justify-center gap-2">
        <Button
          variant="primary"
          data-testid="sight-singing-finish"
          onClick={handleFinish}
          aria-label="Finish sight-singing"
        >
          Finish
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
