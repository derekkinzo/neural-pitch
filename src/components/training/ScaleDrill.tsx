// ScaleDrill — ear-training, scale identification.
//
// Plays an ascending 7-note scale; the user picks the church mode.
//

import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type KeyboardEvent,
  type ReactNode,
} from "react";
import { Button } from "@/components/ui/Button";
import { playDrillPrompt } from "@/lib/drill-synth";
import { handleRadioGroupKeydown } from "@/lib/radiogroup-keys";
import { useSettingsStore } from "@/stores/settingsStore";
import { useTrainingStore } from "@/stores/trainingStore";

interface ModeDef {
  readonly id: string;
  readonly label: string;
  readonly steps: readonly number[];
}

const MODES: ReadonlyArray<ModeDef> = [
  { id: "ionian", label: "Ionian", steps: [0, 2, 4, 5, 7, 9, 11] },
  { id: "dorian", label: "Dorian", steps: [0, 2, 3, 5, 7, 9, 10] },
  { id: "phrygian", label: "Phrygian", steps: [0, 1, 3, 5, 7, 8, 10] },
  { id: "lydian", label: "Lydian", steps: [0, 2, 4, 6, 7, 9, 11] },
  { id: "mixolydian", label: "Mixolydian", steps: [0, 2, 4, 5, 7, 9, 10] },
  { id: "aeolian", label: "Aeolian", steps: [0, 2, 3, 5, 7, 8, 10] },
  { id: "locrian", label: "Locrian", steps: [0, 1, 3, 5, 6, 8, 10] },
];

const ROOT_MIDI = 60;
const TOTAL_PROMPTS = 10;

function pickPromptIndex(promptNumber: number): number {
  return (promptNumber * 3) % MODES.length;
}

export function ScaleDrill(): ReactNode {
  const a4Hz = useSettingsStore((s) => s.a4Hz);
  const scoreAnswer = useTrainingStore((s) => s.scoreAnswer);
  const completeSession = useTrainingStore((s) => s.completeSession);
  const session = useTrainingStore((s) => s.currentSession);

  const [promptNumber, setPromptNumber] = useState<number>(1);
  const [resultText, setResultText] = useState<string | null>(null);
  const [finalText, setFinalText] = useState<string | null>(null);
  const [selectedModeId, setSelectedModeId] = useState<string | null>(null);
  const toastTimeoutRef = useRef<number | null>(null);
  const headingRef = useRef<HTMLHeadingElement | null>(null);
  const radiogroupRef = useRef<HTMLDivElement | null>(null);

  const promptIdx = pickPromptIndex(promptNumber);
  const promptMode = MODES[promptIdx] ?? MODES[0]!;

  const playPrompt = useCallback(() => {
    const notes = promptMode.steps.map((s) => ROOT_MIDI + s);
    playDrillPrompt({
      midiNotes: notes,
      a4Hz,
      noteDurationS: 0.35,
      gapS: 0,
      mode: "sequential",
    });
  }, [a4Hz, promptMode]);

  useEffect(() => {
    playPrompt();
  }, [playPrompt]);

  useEffect(() => {
    return () => {
      if (toastTimeoutRef.current !== null) {
        window.clearTimeout(toastTimeoutRef.current);
      }
    };
  }, []);

  // Mount focus → heading for AT users.
  useEffect(() => {
    headingRef.current?.focus();
  }, []);

  const handleChoice = (modeId: string): void => {
    const correct = modeId === promptMode.id;
    setSelectedModeId(modeId);
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
    setPromptNumber((n) => n + 1);
    setSelectedModeId(null);
  };

  return (
    <section
      data-testid="scale-drill"
      aria-labelledby="scale-drill-heading"
      className="flex w-full max-w-2xl flex-col gap-6"
    >
      <div className="flex items-center justify-between">
        <h2
          ref={headingRef}
          id="scale-drill-heading"
          tabIndex={-1}
          className="text-lg font-semibold text-slate-100 outline-none focus-visible:ring-2 focus-visible:ring-cyan-400"
        >
          Scale prompt {Math.min(TOTAL_PROMPTS, (session?.answered ?? 0) + 1)} of {TOTAL_PROMPTS}
        </h2>
        <span aria-live="polite" className="text-xs text-slate-400">
          Score: {session?.correctCount ?? 0}/{session?.answered ?? 0}
        </span>
      </div>

      <div className="flex flex-col items-center gap-3">
        <Button
          variant="primary"
          data-testid="prompt-play"
          onClick={playPrompt}
          aria-label="Play scale prompt"
        >
          Play prompt
        </Button>
      </div>

      <div
        ref={radiogroupRef}
        role="radiogroup"
        aria-label="Scale choices"
        onKeyDown={(e: KeyboardEvent<HTMLDivElement>) =>
          handleRadioGroupKeydown(e, radiogroupRef.current)
        }
        className="grid grid-cols-2 gap-2 sm:grid-cols-4"
      >
        {MODES.map((m, i) => (
          <button
            key={m.id}
            role="radio"
            type="button"
            aria-checked={selectedModeId === m.id}
            tabIndex={selectedModeId === m.id || (selectedModeId === null && i === 0) ? 0 : -1}
            data-testid={`scale-choice-${m.id}`}
            onClick={() => handleChoice(m.id)}
            className="rounded-md border border-slate-700 bg-slate-900/40 px-3 py-2 text-sm text-slate-100 hover:bg-slate-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400 aria-checked:border-cyan-400 aria-checked:bg-slate-800"
          >
            {m.label}
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
