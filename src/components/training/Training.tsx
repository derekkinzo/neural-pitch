// Training — ear-training landing + drill router.
//
// Mounted as a sibling of the Tuner's `<main>` from `App.tsx`. The Practice
// header button sets `tunerStore.view = "training"` and the Tuner yields its
// main column to this screen. The hash-based deep-link (`/#training`) is
// consumed by App.tsx on mount so Playwright can open the screen directly.
//
// This component carries:
//   - The 5-card landing grid (Drills → "last attempt" stats).
//   - The active-drill router (Intervals / Chords / Scales / SightSinging /
//     TuningPractice). Each drill is its own single-screen component.
//   - A per-drill back affordance (`training-back`) that returns to the
//     landing without flipping the top-level view.
//

import { useEffect, useMemo, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { Button } from "@/components/ui/Button";
import { SettingsDrawer } from "@/components/SettingsDrawer";
import { ChordDrill } from "@/components/training/ChordDrill";
import { IntervalDrill } from "@/components/training/IntervalDrill";
import { ScaleDrill } from "@/components/training/ScaleDrill";
import { SightSingingDrill } from "@/components/training/SightSingingDrill";
import { TuningPracticeDrill } from "@/components/training/TuningPracticeDrill";
import { closeDrillSynth } from "@/lib/drill-synth";
import { formatRelativeLong } from "@/lib/duration-format";
import { selectLatestAttempt, useTrainingStore } from "@/stores/trainingStore";
import { useTunerStore } from "@/stores/tunerStore";
import { DRILLS, type Drill, type DrillAttempt } from "@/types/training";

const SETTINGS_GLYPH = "⚙";

/** Best-effort hydration from the persistent IPC. Hydration silently
 *  falls back to the localStorage-only history when the
 *  `list_drill_history` handler is missing or returns a non-array shape;
 *  the E2E harness's `installTrainingMock` registers the handler so
 *  specs can seed history under the same path the production store
 *  would read. */
async function fetchSeedHistory(): Promise<readonly DrillAttempt[] | null> {
  try {
    const rows = await invoke<unknown>("list_drill_history", {});
    return Array.isArray(rows) ? (rows as DrillAttempt[]) : null;
  } catch {
    return null;
  }
}

/** Optional override for `Date.now()` used by the relative-time copy on
 *  drill cards. Tests pin both spec and page sides to the same epoch
 *  by writing to `__neuralPitchTestHooks.now`; production reads
 *  `undefined` and falls back to `Date.now()`. */
function readTestNow(): number | undefined {
  if (typeof window === "undefined") return undefined;
  const w = window as Window & { __neuralPitchTestHooks?: { now?: number } };
  return w.__neuralPitchTestHooks?.now;
}

export function Training(): ReactNode {
  const setView = useTunerStore((s) => s.setView);
  const currentDrill = useTrainingStore((s) => s.currentDrill);
  const beginSession = useTrainingStore((s) => s.beginSession);
  const abortSession = useTrainingStore((s) => s.abortSession);
  const setHistory = useTrainingStore((s) => s.setHistory);

  const [settingsOpen, setSettingsOpen] = useState<boolean>(false);

  // Hydrate from the IPC on mount. The E2E mock returns the TS-shaped seed
  // history; production swallows the missing/mismatched handler.
  // Pass `{persist: false}` so a stale/empty IPC response cannot wipe the
  // on-disk client cache — the localStorage write only happens when the
  // user actually completes a session via `recordAttempt`/`completeSession`.
  useEffect(() => {
    let cancelled = false;
    void fetchSeedHistory().then((rows) => {
      if (cancelled || rows === null) return;
      setHistory(rows, { persist: false });
    });
    return () => {
      cancelled = true;
    };
  }, [setHistory]);

  // Release the cached AudioContext when the Training screen unmounts so
  // a desktop user navigating between Tuner / Library / Training does not
  // leak a context per visit (Chrome caps live contexts at 6).
  useEffect(() => {
    return () => {
      closeDrillSynth();
    };
  }, []);

  if (currentDrill !== null) {
    return (
      <main className="flex min-h-screen flex-col bg-slate-950 text-slate-50">
        <header className="flex items-center justify-between px-6 py-4">
          <Button
            variant="ghost"
            data-testid="training-back"
            onClick={() => abortSession()}
            aria-label="Back to drills"
          >
            ← Back
          </Button>
          <h1 className="text-lg font-semibold text-slate-100">{currentDrill.title}</h1>
          <Button
            variant="ghost"
            data-testid="training-exit"
            onClick={() => {
              abortSession();
              setView("tuner");
            }}
            aria-label="Exit ear-training"
          >
            Exit
          </Button>
        </header>
        <section className="flex flex-1 flex-col items-center px-6 py-4">
          <ActiveDrill drill={currentDrill} />
        </section>
      </main>
    );
  }

  const onStart = (drill: Drill): void => {
    // Sight-singing and tuning practice run a single continuous prompt;
    // the others use a 10-prompt session.
    const promptCount = drill.id === "sight-singing" || drill.id === "tuning" ? 1 : 10;
    beginSession(drill, promptCount);
  };

  return (
    <>
      <main
        data-testid="training-landing"
        className="flex min-h-screen flex-col bg-slate-950 text-slate-50"
      >
        <header className="flex items-center justify-between px-6 py-4">
          <h1 className="text-lg font-semibold text-slate-100">Ear-training drills</h1>
          <div className="flex items-center gap-2">
            <Button
              variant="ghost"
              data-testid="settings-trigger"
              onClick={() => setSettingsOpen(true)}
              aria-label="Open settings"
            >
              <span aria-hidden="true" className="text-lg">
                {SETTINGS_GLYPH}
              </span>
            </Button>
            <Button
              variant="ghost"
              data-testid="training-exit"
              onClick={() => setView("tuner")}
              aria-label="Exit ear-training"
            >
              ← Tuner
            </Button>
          </div>
        </header>

        <section className="flex flex-1 flex-col items-center px-6 py-6">
          <div
            role="list"
            aria-label="Available drills"
            className="grid w-full max-w-3xl grid-cols-1 gap-4 sm:grid-cols-2 lg:grid-cols-3"
          >
            {DRILLS.map((drill) => (
              <DrillCard key={drill.id} drill={drill} onStart={() => onStart(drill)} />
            ))}
          </div>
        </section>
      </main>

      <SettingsDrawer open={settingsOpen} onOpenChange={setSettingsOpen} />
    </>
  );
}

interface DrillCardProps {
  drill: Drill;
  onStart: () => void;
}

function DrillCard({ drill, onStart }: DrillCardProps): ReactNode {
  const latest = useTrainingStore((s) => selectLatestAttempt(s, drill.id));
  // Pin "now" once per render so the cards do not re-render every second
  // chasing the relative-time labels. Re-mounting the landing (back from a
  // drill) recomputes naturally. Tests can override via the
  // `__neuralPitchTestHooks.now` slot to pin both spec and page sides to
  // the same epoch and remove wall-clock drift from the relative-time
  // assertions.
  const now = useMemo(() => readTestNow() ?? Date.now(), []);

  const accuracyText = latest === null ? "—" : `${Math.round(latest.accuracy * 100)}%`;
  const timeText = latest === null ? "—" : formatRelativeLong(latest.completedAt, now);

  return (
    <article
      role="listitem"
      data-testid="drill-card"
      data-drill-id={drill.id}
      className="flex flex-col gap-3 rounded-md border border-slate-700 bg-slate-900/60 p-4 text-sm text-slate-200"
    >
      <div className="flex flex-col gap-1">
        <h2 className="text-base font-semibold text-slate-100">{drill.title}</h2>
        <p className="text-xs leading-relaxed text-slate-400">{drill.description}</p>
      </div>
      <dl className="grid grid-cols-2 gap-1 text-xs text-slate-400">
        <dt className="text-slate-400">Last attempt accuracy</dt>
        <dd className="text-right text-slate-200" data-testid="drill-card-accuracy">
          {accuracyText}
        </dd>
        <dt className="text-slate-400">Last attempt</dt>
        <dd className="text-right text-slate-200" data-testid="drill-card-when">
          {timeText}
        </dd>
      </dl>
      <Button variant="primary" onClick={onStart} aria-label={`Start ${drill.title}`}>
        Start
      </Button>
    </article>
  );
}

interface ActiveDrillProps {
  drill: Drill;
}

function ActiveDrill({ drill }: ActiveDrillProps): ReactNode {
  switch (drill.id) {
    case "intervals":
      return <IntervalDrill />;
    case "chords":
      return <ChordDrill />;
    case "scales":
      return <ScaleDrill />;
    case "sight-singing":
      return <SightSingingDrill />;
    case "tuning":
      return <TuningPracticeDrill />;
    default:
      return null;
  }
}
