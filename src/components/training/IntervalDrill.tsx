// IntervalDrill — ear-training, interval recognition.
//
// Single-screen flow:
//   1. Pick a deterministic-but-shuffled prompt from the 12 interval
//      candidates (m2..P8).
//   2. Synthesise the prompt via `playDrillPrompt({ midiNotes, mode: "sequential" })`.
//   3. Render a 12-radio choice grid; on click, call
//      `trainingStore.scoreAnswer({ correct })` and surface a result toast.
//   4. After N=10 prompts the final-score toast lingers and the back
//      affordance returns to the landing.
//
// Choice labels honour `settingsStore.noteLabelMode`:
//   - letter      → "P5", "M3", … (standard interval-quality tokens)
//   - movable-do  → "Sol", "Mi", … (solfege relative to the drill's tonic)
//   - fixed-do    → solfege anchored to C
//

import {
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { Button } from "@/components/ui/Button";
import { playDrillPrompt } from "@/lib/drill-synth";
import { formatIntervalLabel } from "@/lib/note-format";
import { handleRadioGroupKeydown } from "@/lib/radiogroup-keys";
import { useSettingsStore } from "@/stores/settingsStore";
import { useTrainingStore } from "@/stores/trainingStore";

const INTERVAL_SEMITONES = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12] as const;

/** Default tonic for the drill — middle C. Solfege rendering anchors here
 *  in movable-do mode; in letter mode the tonic is irrelevant. */
const DEFAULT_TONIC_MIDI = 60;

const TOTAL_PROMPTS = 10;

interface PromptState {
  /** Index in INTERVAL_SEMITONES of the correct answer. */
  readonly correctIndex: number;
  /** Counter of prompts shown so far this session (1-based). */
  readonly promptNumber: number;
}

function pickNextPrompt(promptNumber: number): PromptState {
  // Deterministic-ish picker: cycles through the candidates with a
  // golden-ratio step so the same answer does not repeat back-to-back.
  // This avoids importing a PRNG and keeps the drill flow reproducible
  // for the test harness.
  const step = 7;
  const correctIndex = (promptNumber * step) % INTERVAL_SEMITONES.length;
  return { correctIndex, promptNumber };
}

export function IntervalDrill(): ReactNode {
  const noteLabelMode = useSettingsStore((s) => s.noteLabelMode);
  const a4Hz = useSettingsStore((s) => s.a4Hz);
  const scoreAnswer = useTrainingStore((s) => s.scoreAnswer);
  const completeSession = useTrainingStore((s) => s.completeSession);
  const session = useTrainingStore((s) => s.currentSession);

  const [prompt, setPrompt] = useState<PromptState>(() => pickNextPrompt(1));
  const [resultText, setResultText] = useState<string | null>(null);
  const [finalText, setFinalText] = useState<string | null>(null);
  const [selectedSemitones, setSelectedSemitones] = useState<number | null>(null);
  const toastTimeoutRef = useRef<number | null>(null);
  const headingRef = useRef<HTMLHeadingElement | null>(null);
  const radiogroupRef = useRef<HTMLDivElement | null>(null);

  const correctSemitones = INTERVAL_SEMITONES[prompt.correctIndex] ?? 7;

  const playCurrentPrompt = useCallback(() => {
    playDrillPrompt({
      midiNotes: [DEFAULT_TONIC_MIDI, DEFAULT_TONIC_MIDI + correctSemitones],
      a4Hz,
      noteDurationS: 0.55,
      gapS: 0.05,
      mode: "sequential",
    });
  }, [a4Hz, correctSemitones]);

  // Auto-play on mount (and on each prompt advance) so AT users do not
  // need to find the play button before the first prompt.
  useEffect(() => {
    playCurrentPrompt();
  }, [playCurrentPrompt]);

  useEffect(() => {
    return () => {
      if (toastTimeoutRef.current !== null) {
        window.clearTimeout(toastTimeoutRef.current);
      }
    };
  }, []);

  const handleChoice = (semitones: number): void => {
    const correct = semitones === correctSemitones;
    setSelectedSemitones(semitones);
    scoreAnswer({ correct });
    setResultText(correct ? "Correct" : "Incorrect");
    if (toastTimeoutRef.current !== null) {
      window.clearTimeout(toastTimeoutRef.current);
    }
    toastTimeoutRef.current = window.setTimeout(() => {
      setResultText(null);
      toastTimeoutRef.current = null;
    }, 1500);

    const answered = (session?.answered ?? 0) + 1;
    if (answered >= TOTAL_PROMPTS) {
      const attempt = completeSession();
      if (attempt !== null) {
        const pct = Math.round(attempt.accuracy * 100);
        setFinalText(
          `Session complete: ${attempt.correctCount} of ${attempt.totalPrompts} correct (${pct}%).`,
        );
      }
      return;
    }
    setPrompt(pickNextPrompt(answered + 1));
    setSelectedSemitones(null);
  };

  // Move keyboard focus to the heading on mount so AT users land on
  // the drill title instead of the unmounted Start button.
  useEffect(() => {
    headingRef.current?.focus();
  }, []);

  const choices = useMemo(
    () =>
      INTERVAL_SEMITONES.map((semitones) => ({
        semitones,
        label: formatIntervalLabel(semitones, noteLabelMode, DEFAULT_TONIC_MIDI),
      })),
    [noteLabelMode],
  );

  return (
    <section
      data-testid="interval-drill"
      aria-labelledby="interval-drill-heading"
      className="flex w-full max-w-2xl flex-col gap-6"
    >
      <div className="flex items-center justify-between">
        <h2
          ref={headingRef}
          id="interval-drill-heading"
          tabIndex={-1}
          className="text-lg font-semibold text-slate-100 outline-none focus-visible:ring-2 focus-visible:ring-cyan-400"
        >
          Interval prompt {Math.min(TOTAL_PROMPTS, (session?.answered ?? 0) + 1)} of {TOTAL_PROMPTS}
        </h2>
        <span aria-live="polite" className="text-xs text-slate-400">
          Score: {session?.correctCount ?? 0}/{session?.answered ?? 0}
        </span>
      </div>

      <div className="flex flex-col items-center gap-3">
        <Button
          variant="primary"
          data-testid="prompt-play"
          onClick={playCurrentPrompt}
          aria-label="Play interval prompt"
        >
          Play prompt
        </Button>
        <p className="text-xs text-slate-400">Listen, then choose the matching interval.</p>
      </div>

      <div
        ref={radiogroupRef}
        role="radiogroup"
        aria-label="Interval choices"
        onKeyDown={(e: KeyboardEvent<HTMLDivElement>) =>
          handleRadioGroupKeydown(e, radiogroupRef.current)
        }
        className="grid grid-cols-3 gap-2 sm:grid-cols-4"
      >
        {choices.map((c, i) => (
          <button
            key={c.semitones}
            role="radio"
            type="button"
            aria-checked={selectedSemitones === c.semitones}
            tabIndex={
              selectedSemitones === c.semitones || (selectedSemitones === null && i === 0) ? 0 : -1
            }
            data-testid={`interval-choice-${c.semitones}`}
            onClick={() => handleChoice(c.semitones)}
            className="rounded-md border border-slate-700 bg-slate-900/40 px-3 py-2 text-sm text-slate-100 hover:bg-slate-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400 aria-checked:border-cyan-400 aria-checked:bg-slate-800"
          >
            {c.label}
          </button>
        ))}
      </div>

      {resultText !== null ? (
        <div
          role="status"
          aria-live="polite"
          data-testid="drill-result-toast"
          className="self-center rounded-md border border-cyan-500/40 bg-slate-900/95 px-4 py-2 text-sm text-slate-100 shadow"
        >
          {resultText}
        </div>
      ) : null}

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
