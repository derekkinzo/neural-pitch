// StemSeparationPanel — four-stem HTDemucs separation affordance.
//
// Mounts inside RecordingDetail directly below TranscribePanel and above
// the Range / Vibrato readout grid. Four render branches driven by
// `stemsStore.perRecording[id].status` (the progress branch is shared
// between the `downloading-model` and `separating` FSM states with a
// different sub-label):
//
//   1. idle              — primary "Separate stems" button.
//   2. progress          — `<progress role="progressbar">` rendered for
//                          either the `downloading-model` state
//                          ("Downloading HTDemucs (~80 MB)" sub-label,
//                          no Cancel button) or the `separating` state
//                          (per-stage sub-label, Cancel button +
//                          Escape shortcut). aria-valuetext rebuilds
//                          per-stage from a rAF loop reading the ref bus.
//   3. complete          — vertical stack of four StemCards (Vocals,
//                          Drums, Bass, Other) inside `role="list"`.
//   4. error             — `role="alert"` paragraph + Retry button.
//
// The hot-path `percent` value is published at ~10–20 Hz from Rust. The
// panel reads percent through `readStemPercent(id)` inside a rAF loop
// and writes the `<progress>` element's `value` plus aria attributes
// imperatively — React DOES NOT re-render per frame. Stage transitions
// re-render the label only (~5 transitions per separation).
//
//   src/components/recordings/TranscribePanel.tsx (idle / progress / complete branches precedent)

import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { PlaybackPanel } from "@/components/recordings/PlaybackPanel";
import {
  readStemPercent,
  selectStemState,
  stageDisplayLabel,
  stemsAriaValueText,
  useStemsStore,
} from "@/stores/stemsStore";
import { useTranscriptionStore } from "@/stores/transcriptionStore";
import type { RecordingId } from "@/types/recording";
import {
  STEM_DISPLAY_LABEL,
  STEM_KIND_ORDER,
  type SeparateStage,
  type StemKind,
} from "@/types/stems";

export interface StemSeparationPanelProps {
  recordingId: RecordingId;
  /** Display label shown in the Save dialog default filename. */
  recordingLabel: string;
}

/** Read the `prefers-reduced-motion: reduce` media query and subscribe
 *  to its change events so an OS-level preference flip takes effect
 *  without requiring a remount. */
function useReducedMotion(): boolean {
  const [reduced, setReduced] = useState<boolean>(() => {
    if (typeof window === "undefined") return false;
    try {
      return window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    } catch {
      return false;
    }
  });
  useEffect(() => {
    if (typeof window === "undefined") return;
    let mq: MediaQueryList;
    try {
      mq = window.matchMedia("(prefers-reduced-motion: reduce)");
    } catch {
      return;
    }
    setReduced(mq.matches);
    const onChange = (e: MediaQueryListEvent): void => setReduced(e.matches);
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);
  return reduced;
}

export function StemSeparationPanel({
  recordingId,
  recordingLabel,
}: StemSeparationPanelProps): ReactNode {
  const state = useStemsStore((s) => selectStemState(s, recordingId));
  const activeProgress = useStemsStore((s) => s.activeProgress);
  // Prefer the per-recording live status so concurrent separations on
  // two different recordings cannot interleave their stage
  // announcements. Falls back to the global slot for older code paths.
  const liveStatus = useStemsStore((s) => s.liveStatusByRecording.get(recordingId) ?? s.liveStatus);
  const separate = useStemsStore((s) => s.separate);
  const cancel = useStemsStore((s) => s.cancel);

  const reducedMotion = useReducedMotion();

  // Hot-path progress bar refs. We update the DOM imperatively from a
  // rAF loop so the per-frame percent never enters React's reconciler.
  const progressRef = useRef<HTMLProgressElement | null>(null);
  const lastWrittenPercentRef = useRef<number>(-1);

  const status = state.status;
  const isProgressBranch = status === "separating" || status === "downloading-model";
  const isSeparating = status === "separating";

  // Pull the active stage for this recording — only that label drives
  // re-renders; the percent flows through the ref bus.
  const stage: SeparateStage = useMemo(() => {
    if (activeProgress?.recordingId === recordingId) return activeProgress.stage;
    return "vocals";
  }, [activeProgress, recordingId]);

  // rAF loop: read the ref-bus percent and write directly to the
  // `<progress>` element + aria attributes. Cheaper than React state
  // updates because it bypasses reconciliation entirely.
  useEffect(() => {
    if (!isProgressBranch) return;
    let raf = 0;
    const loop = (): void => {
      const el = progressRef.current;
      if (el !== null) {
        const pct = Math.round(readStemPercent(recordingId));
        if (pct !== lastWrittenPercentRef.current) {
          lastWrittenPercentRef.current = pct;
          el.value = pct;
          el.setAttribute("aria-valuenow", String(pct));
          el.setAttribute("aria-valuetext", stemsAriaValueText(stage, pct));
        }
      }
      raf = requestAnimationFrame(loop);
    };
    raf = requestAnimationFrame(loop);
    return () => cancelAnimationFrame(raf);
  }, [isProgressBranch, recordingId, stage]);

  // When the stage changes the label moves; force a one-shot ARIA write
  // so the screen-reader sees the new "Separating drums" copy even if
  // the percent ref hasn't ticked since the previous stage's last frame.
  useEffect(() => {
    const el = progressRef.current;
    if (el === null) return;
    const pct = Math.round(readStemPercent(recordingId));
    el.setAttribute("aria-valuetext", stemsAriaValueText(stage, pct));
  }, [stage, recordingId]);

  // Reset the imperative cache when leaving the progress branch so the
  // next separation starts at 0 instead of the last-rendered percent.
  useEffect(() => {
    if (!isProgressBranch) lastWrittenPercentRef.current = -1;
  }, [isProgressBranch]);

  // Focus management: when the panel transitions to `complete`, move
  // focus to the first card heading so a keyboard / AT user lands on
  // the new content. The h3s carry tabIndex={-1} to accept programmatic
  // focus without entering tab order.
  const firstStemHeadingRef = useRef<HTMLHeadingElement | null>(null);
  useEffect(() => {
    if (status === "complete") {
      const h = firstStemHeadingRef.current;
      if (h !== null) {
        try {
          h.focus({ preventScroll: true });
        } catch {
          /* swallow: focus can throw if the element was unmounted between
             the effect schedule and the run. */
        }
      }
    }
  }, [status]);

  // Escape-to-cancel while separating. Bound to the panel root so the
  // shortcut is scoped — pressing Escape elsewhere does not abort.
  const onPanelKeyDown = (e: React.KeyboardEvent<HTMLElement>): void => {
    if (e.key !== "Escape") return;
    if (!isSeparating) return;
    e.preventDefault();
    cancel(recordingId);
  };

  const onSeparate = (): void => {
    void separate(recordingId);
  };

  const onCancel = (): void => {
    cancel(recordingId);
  };

  const onRetry = (): void => {
    void separate(recordingId);
  };

  return (
    <section
      data-testid="stem-separation-panel"
      role="group"
      aria-label="Stems"
      aria-busy={isProgressBranch}
      onKeyDown={onPanelKeyDown}
      className="flex flex-col gap-2 rounded-md border border-slate-700 bg-slate-900/40 p-3"
    >
      <div className="flex items-center justify-between gap-2">
        <span className="text-xs font-semibold uppercase tracking-wide text-slate-300">Stems</span>
      </div>

      {/* Polite live region — only stage transitions write here. The
          per-frame percent does NOT, so AT speech does not churn.
          Carries the latest stage / completion / cancellation copy so
          AT users re-querying after a transition still get context. */}
      <div role="status" aria-live="polite" data-testid="stems-status" className="sr-only">
        {liveStatus ?? ""}
      </div>

      {status === "idle" ? (
        <IdleBranch onSeparate={onSeparate} />
      ) : status === "downloading-model" ? (
        <ProgressBranch
          progressRef={progressRef}
          subLabel="Downloading HTDemucs (~80 MB)"
          stageLabel="Downloading separation model"
          onCancel={onCancel}
          showCancel={false}
          reducedMotion={reducedMotion}
        />
      ) : status === "separating" ? (
        <ProgressBranch
          progressRef={progressRef}
          subLabel={stageDisplayLabel(stage)}
          stageLabel={stageDisplayLabel(stage)}
          onCancel={onCancel}
          showCancel={true}
          reducedMotion={reducedMotion}
        />
      ) : status === "complete" ? (
        <CompleteBranch
          recordingId={recordingId}
          recordingLabel={recordingLabel}
          stemPaths={state.stemPaths ?? null}
          firstStemHeadingRef={firstStemHeadingRef}
        />
      ) : (
        <ErrorBranch error={state.error} onRetry={onRetry} />
      )}
    </section>
  );
}

interface IdleBranchProps {
  onSeparate: () => void;
}

function IdleBranch({ onSeparate }: IdleBranchProps): ReactNode {
  return (
    <div>
      <button
        type="button"
        data-testid="separate-stems"
        onClick={onSeparate}
        className={[
          "rounded-md border border-cyan-500/40 bg-cyan-500/10 px-3 py-1 text-xs font-medium text-cyan-200",
          "hover:bg-cyan-500/20 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400",
        ].join(" ")}
      >
        Separate stems
      </button>
    </div>
  );
}

interface ProgressBranchProps {
  progressRef: React.RefObject<HTMLProgressElement | null>;
  subLabel: string;
  stageLabel: string;
  onCancel: () => void;
  showCancel: boolean;
  reducedMotion: boolean;
}

function ProgressBranch({
  progressRef,
  subLabel,
  stageLabel,
  onCancel,
  showCancel,
  reducedMotion,
}: ProgressBranchProps): ReactNode {
  return (
    <div className="flex flex-col gap-2">
      <progress
        ref={progressRef}
        data-testid="stems-progress"
        role="progressbar"
        aria-label="Stem separation progress"
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={0}
        aria-valuetext={stemsAriaValueText("vocals", 0)}
        max={100}
        value={0}
        className="h-2 w-full overflow-hidden rounded-full bg-slate-800"
      />
      <div className="flex items-center justify-between gap-2">
        <span className="text-xs text-slate-400">{reducedMotion ? "Working…" : subLabel}</span>
        {/* Off-screen stage name for AT users — kept readable so a
            reduced-motion user (whose visible label collapses to
            "Working…") still hears the per-stage name. */}
        <span className="sr-only">{stageLabel}</span>
        {showCancel ? (
          <button
            type="button"
            data-testid="cancel-separation"
            aria-keyshortcuts="Escape"
            onClick={onCancel}
            className={[
              "rounded-md border border-slate-600 bg-slate-800 px-3 py-1 text-xs text-slate-200",
              "hover:bg-slate-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400",
            ].join(" ")}
          >
            Cancel
          </button>
        ) : null}
      </div>
    </div>
  );
}

interface CompleteBranchProps {
  recordingId: RecordingId;
  recordingLabel: string;
  stemPaths: Readonly<Record<StemKind, string>> | null;
  firstStemHeadingRef: React.RefObject<HTMLHeadingElement | null>;
}

function CompleteBranch({
  recordingId,
  recordingLabel,
  stemPaths,
  firstStemHeadingRef,
}: CompleteBranchProps): ReactNode {
  return (
    <ul data-testid="stems-list" role="list" aria-label="Stems" className="flex flex-col gap-2">
      {STEM_KIND_ORDER.map((kind, idx) => (
        <li key={kind} role="listitem" data-testid={`stem-card-${kind}`}>
          <StemCard
            recordingId={recordingId}
            recordingLabel={recordingLabel}
            kind={kind}
            audioPath={stemPaths?.[kind] ?? ""}
            headingRef={idx === 0 ? firstStemHeadingRef : undefined}
          />
        </li>
      ))}
    </ul>
  );
}

interface StemCardProps {
  recordingId: RecordingId;
  recordingLabel: string;
  kind: StemKind;
  audioPath: string;
  headingRef?: React.RefObject<HTMLHeadingElement | null> | undefined;
}

function StemCard({
  recordingId,
  recordingLabel,
  kind,
  audioPath,
  headingRef,
}: StemCardProps): ReactNode {
  const transcribe = useTranscriptionStore((s) => s.transcribe);
  const [exportError, setExportError] = useState<string | null>(null);

  const onTranscribe = (): void => {
    void transcribe(recordingId, { stemKind: kind });
  };

  const onExport = (): void => {
    // Resolve the destination via the dialog plugin in production; the
    // E2E mock exports `export_stem` directly with a sentinel dest so
    // the call flow is observable. Mirrors the export-MIDI pattern.
    setExportError(null);
    void (async (): Promise<void> => {
      const safeLabel = recordingLabel.replace(/\.[a-z0-9]+$/i, "");
      const destPath = `${safeLabel}-${kind}.flac`;
      try {
        await invoke<number>("export_stem", {
          recordingId,
          stemKind: kind,
          destPath,
        });
      } catch (err: unknown) {
        // Surface the failure inline so the user sees something other
        // than a silent no-op. The Rust shell already logs server-side.
        const msg = err instanceof Error ? err.message : String(err);
        setExportError(msg);
      }
    })();
  };

  return (
    <section
      role="group"
      aria-label={`${STEM_DISPLAY_LABEL[kind]} stem`}
      className="flex flex-col gap-2 rounded-md border border-slate-700 bg-slate-900/30 p-2"
    >
      <h3
        ref={headingRef}
        tabIndex={-1}
        className="text-sm font-semibold text-slate-100 outline-none focus-visible:ring-2 focus-visible:ring-cyan-400"
      >
        {STEM_DISPLAY_LABEL[kind]}
      </h3>
      {audioPath !== "" ? <PlaybackPanel audioPath={audioPath} variant="stem" /> : null}
      <div className="flex flex-wrap items-center gap-2">
        <button
          type="button"
          data-testid={`transcribe-stem-${kind}`}
          onClick={onTranscribe}
          className={[
            "rounded-md border border-slate-600 bg-slate-800 px-3 py-1 text-xs text-slate-200",
            "hover:bg-slate-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400",
          ].join(" ")}
        >
          Transcribe this stem
        </button>
        <button
          type="button"
          data-testid={`export-stem-${kind}`}
          onClick={onExport}
          className={[
            "rounded-md border border-slate-600 bg-slate-800 px-3 py-1 text-xs text-slate-200",
            "hover:bg-slate-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400",
          ].join(" ")}
        >
          Export FLAC
        </button>
      </div>
      {exportError !== null ? (
        <p role="alert" data-testid={`stem-export-error-${kind}`} className="text-xs text-rose-300">
          Export failed: {exportError}
        </p>
      ) : null}
    </section>
  );
}

interface ErrorBranchProps {
  error: string | undefined;
  onRetry: () => void;
}

function ErrorBranch({ error, onRetry }: ErrorBranchProps): ReactNode {
  return (
    <div className="flex flex-col gap-2">
      <p role="alert" className="text-xs text-rose-300">
        Stem separation failed: {error ?? "unknown error"}
      </p>
      <div>
        <button
          type="button"
          data-testid="stems-retry"
          onClick={onRetry}
          className={[
            "rounded-md border border-slate-600 bg-slate-800 px-3 py-1 text-xs text-slate-200",
            "hover:bg-slate-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400",
          ].join(" ")}
        >
          Retry
        </button>
      </div>
    </div>
  );
}
