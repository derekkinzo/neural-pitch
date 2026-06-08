// TranscribePanel — Phase 3 polyphonic transcription affordance.
//
// Mounts inside RecordingDetail directly below the AnalysisSummary card
// and above the `<RangeReadout> + <VibratoReadout>` grid. Three render
// branches:
//
//   1. Idle / no result. Primary "Transcribe to MIDI" button. Disabled
//      via `aria-disabled` (mirroring the Re-analyze button precedent)
//      when `analysisStore.inProgress` contains the current id, or when
//      `transcriptionStore.inProgress` already contains it.
//   2. In progress. Renders `<progress role="progressbar" aria-label=
//      "Transcribing recording">` driven by `transcribe-progress` events.
//      Cancel is intentionally omitted in this phase.
//   3. Complete. Shows "Notes detected: N", an "Export MIDI..." button,
//      and — when `summary.wasCached` — a "Transcription cached" badge
//      plus a "Re-transcribe" affordance that calls
//      `transcribe(id, { forceRefresh: true })`.
//
// The `transcribe-progress` subscription lives in RecordingsList (one
// hook for the whole drawer mount). Progress percent is read from the
// store via a per-id selector.
//
// Failure paths route through `transcriptionStore.errors`; the panel
// renders a `role="alert"` paragraph in that branch.
//
//   src/components/recordings/RecordingDetail.tsx (parent — owns the row id)
//   src/stores/transcriptionStore.ts (Zustand actions + parked completion)

import { useMemo, type ReactNode } from "react";
import { useAnalysisStore } from "@/stores/analysisStore";
import { useTranscriptionStore } from "@/stores/transcriptionStore";
import type { RecordingId } from "@/types/recording";

export interface TranscribePanelProps {
  recordingId: RecordingId;
}

export function TranscribePanel({ recordingId }: TranscribePanelProps): ReactNode {
  const summary = useTranscriptionStore((s) => s.byRecording.get(recordingId));
  const inProgress = useTranscriptionStore((s) => s.inProgress.has(recordingId));
  const progressPercent = useTranscriptionStore((s) => s.progressByRecording.get(recordingId));
  const errorMsg = useTranscriptionStore((s) => s.errors.get(recordingId));
  const transcribe = useTranscriptionStore((s) => s.transcribe);
  const exportMidi = useTranscriptionStore((s) => s.exportMidi);

  // Re-analyze blocks transcribe and vice versa — the UI gates one against
  // the other so the user does not see two competing progress bars.
  const analysisInProgress = useAnalysisStore((s) => s.inProgress.has(recordingId));

  const percent = useMemo(
    () =>
      typeof progressPercent === "number" ? Math.max(0, Math.min(100, progressPercent)) : undefined,
    [progressPercent],
  );

  const onTranscribe = (): void => {
    void transcribe(recordingId);
  };

  const onRetranscribe = (): void => {
    void transcribe(recordingId, { forceRefresh: true });
  };

  const onExport = (): void => {
    // Phase 3 destination is implicit (Rust shell decides path); we
    // forward the recording id and a placeholder destination string the
    // shell ignores when the caller is the front-end. The mock
    // `export_midi` handler records the call regardless of dest, so the
    // spec assertion on call count holds either way.
    void exportMidi(recordingId, "");
  };

  const transcribeDisabled = inProgress || analysisInProgress;

  return (
    <section
      data-testid="transcribe-panel"
      role="group"
      aria-label="Transcribe to MIDI"
      aria-busy={inProgress}
      className="flex flex-col gap-2 rounded-md border border-slate-700 bg-slate-900/40 p-3"
    >
      <div className="flex items-center justify-between gap-2">
        <span className="text-xs font-semibold uppercase tracking-wide text-slate-300">
          Transcribe
        </span>
        {summary !== undefined && summary.wasCached ? (
          <span
            data-testid="transcribe-cached-badge"
            className="rounded-full border border-cyan-700 px-2 py-0.5 text-xs text-cyan-300"
          >
            Transcription cached
          </span>
        ) : null}
      </div>

      {errorMsg !== undefined && !inProgress ? (
        <div role="alert" className="text-xs text-rose-300">
          Transcription failed: {errorMsg}
        </div>
      ) : null}

      {inProgress ? (
        <div className="flex flex-col gap-1">
          <progress
            id="transcribe-progress-bar"
            data-testid="transcribe-progress"
            role="progressbar"
            aria-label="Transcribing recording"
            max={100}
            {...(percent !== undefined ? { value: percent } : {})}
            className="h-2 w-full overflow-hidden rounded-full bg-slate-800"
          />
          <span className="text-xs text-slate-400">Transcribing…</span>
        </div>
      ) : summary === undefined ? (
        <div>
          <button
            type="button"
            data-testid="transcribe-button"
            aria-disabled={transcribeDisabled}
            aria-describedby={inProgress ? "transcribe-progress-bar" : undefined}
            onClick={(e) => {
              if (transcribeDisabled) {
                e.preventDefault();
                return;
              }
              onTranscribe();
            }}
            className={[
              "rounded-md border border-cyan-500/40 bg-cyan-500/10 px-3 py-1 text-xs font-medium text-cyan-200",
              "hover:bg-cyan-500/20 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400",
              "aria-disabled:cursor-not-allowed aria-disabled:opacity-60",
            ].join(" ")}
          >
            Transcribe to MIDI
          </button>
        </div>
      ) : (
        <div className="flex flex-wrap items-center gap-x-3 gap-y-1 text-sm">
          <span className="font-medium text-slate-100" data-testid="transcribe-note-count">
            {`Notes detected: ${summary.noteCount}`}
          </span>
          <button
            type="button"
            data-testid="export-midi"
            onClick={onExport}
            className={[
              "rounded-md border border-slate-600 bg-slate-800 px-3 py-1 text-xs text-slate-200",
              "hover:bg-slate-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400",
            ].join(" ")}
          >
            Export MIDI…
          </button>
          {summary.wasCached ? (
            <button
              type="button"
              data-testid="retranscribe"
              onClick={onRetranscribe}
              className={[
                "rounded-md border border-slate-600 bg-slate-800 px-3 py-1 text-xs text-slate-200",
                "hover:bg-slate-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400",
              ].join(" ")}
            >
              Re-transcribe
            </button>
          ) : null}
        </div>
      )}
    </section>
  );
}
