// ChordDrill — Phase 4 ear-training, chord-quality recognition.
//
// Three or four notes triggered simultaneously; the user picks the chord
// quality (Major / Minor / Dim / Aug / Maj7 / Dom7 / Min7).
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

interface ChordDef {
  readonly id: string;
  readonly label: string;
  readonly intervals: readonly number[];
}

const CHORDS: ReadonlyArray<ChordDef> = [
  { id: "maj", label: "Major", intervals: [0, 4, 7] },
  { id: "min", label: "Minor", intervals: [0, 3, 7] },
  { id: "dim", label: "Dim", intervals: [0, 3, 6] },
  { id: "aug", label: "Aug", intervals: [0, 4, 8] },
  { id: "maj7", label: "Maj7", intervals: [0, 4, 7, 11] },
  { id: "dom7", label: "Dom7", intervals: [0, 4, 7, 10] },
  { id: "min7", label: "Min7", intervals: [0, 3, 7, 10] },
];

const DEFAULT_ROOT_MIDI = 60;
const TOTAL_PROMPTS = 10;

function pickPromptIndex(promptNumber: number): number {
  return (promptNumber * 5) % CHORDS.length;
}

export function ChordDrill(): ReactNode {
  const a4Hz = useSettingsStore((s) => s.a4Hz);
  const scoreAnswer = useTrainingStore((s) => s.scoreAnswer);
  const completeSession = useTrainingStore((s) => s.completeSession);
  const session = useTrainingStore((s) => s.currentSession);

  const [promptNumber, setPromptNumber] = useState<number>(1);
  const [resultText, setResultText] = useState<string | null>(null);
  const [finalText, setFinalText] = useState<string | null>(null);
  const [selectedChordId, setSelectedChordId] = useState<string | null>(null);
  const toastTimeoutRef = useRef<number | null>(null);
  const headingRef = useRef<HTMLHeadingElement | null>(null);
  const radiogroupRef = useRef<HTMLDivElement | null>(null);

  const promptIdx = pickPromptIndex(promptNumber);
  const promptChord = CHORDS[promptIdx] ?? CHORDS[0]!;

  const playPrompt = useCallback(() => {
    const notes = promptChord.intervals.map((iv) => DEFAULT_ROOT_MIDI + iv);
    playDrillPrompt({
      midiNotes: notes,
      a4Hz,
      noteDurationS: 1.0,
      mode: "parallel",
    });
  }, [a4Hz, promptChord]);

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

  // Mount focus → heading. Same pattern as IntervalDrill.
  useEffect(() => {
    headingRef.current?.focus();
  }, []);

  const handleChoice = (chordId: string): void => {
    const correct = chordId === promptChord.id;
    setSelectedChordId(chordId);
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
    setSelectedChordId(null);
  };

  return (
    <section
      data-testid="chord-drill"
      aria-labelledby="chord-drill-heading"
      className="flex w-full max-w-2xl flex-col gap-6"
    >
      <div className="flex items-center justify-between">
        <h2
          ref={headingRef}
          id="chord-drill-heading"
          tabIndex={-1}
          className="text-lg font-semibold text-slate-100 outline-none focus-visible:ring-2 focus-visible:ring-cyan-400"
        >
          Chord prompt {Math.min(TOTAL_PROMPTS, (session?.answered ?? 0) + 1)} of {TOTAL_PROMPTS}
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
          aria-label="Play chord prompt"
        >
          Play prompt
        </Button>
      </div>

      <div
        ref={radiogroupRef}
        role="radiogroup"
        aria-label="Chord choices"
        onKeyDown={(e: KeyboardEvent<HTMLDivElement>) =>
          handleRadioGroupKeydown(e, radiogroupRef.current)
        }
        className="grid grid-cols-2 gap-2 sm:grid-cols-4"
      >
        {CHORDS.map((c, i) => (
          <button
            key={c.id}
            role="radio"
            type="button"
            aria-checked={selectedChordId === c.id}
            tabIndex={selectedChordId === c.id || (selectedChordId === null && i === 0) ? 0 : -1}
            data-testid={`chord-choice-${c.id}`}
            onClick={() => handleChoice(c.id)}
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
