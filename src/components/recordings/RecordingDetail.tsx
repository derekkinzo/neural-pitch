// RecordingDetail — Phase 2.1 detail panel below RecordingsList.
//
// Mounts inside the Recordings drawer body (sibling of PlaybackPanel) when
// `currentRecordingId` is set. Three vertically stacked regions:
//
//   1. Header (filename / duration / relative createdAt / instrument badge /
//      A4 pill). All values come from the Recording row already in
//      recordingsStore — no extra IPC.
//   2. AnalysisSummary card — median note (e.g. "A4"), median cents (signed,
//      1 decimal), voiced ratio (percentage), wasCached badge ("Cached"
//      vs "Fresh"). While `inProgress` contains the id, the card replaces
//      the numeric readouts with a `<progress role="progressbar">`.
//   3. ContourLine — `<figure>` containing the canvas + visually-hidden
//      `<figcaption>` mirroring the wrapper's aria-label. Voiced spans
//      drawn as a connected polyline; unvoiced spans as gaps.
//
// The Re-analyze button (`[data-testid="reanalyze"]`) calls
// `analyze(id, { forceRefresh: true })`, which marks the id `inProgress`,
// dispatches `analyze_recording`, and parks until an `analysis-progress`
// event with `percent >= 100` arrives — the cache spec drives this with
// `pushAnalysisProgress`.
//
// Cross-references:
//   docs/design/DESIGN.md §7.5 (Phase 2.1 frontend additions)
//   docs/design/DESIGN.md §8.3 (analysis_cache schema)
//   docs/adr/0006-visual-only-feedback-prefers-reduced-motion.md
//   src/components/CentsMeter.tsx (canonical canvas + DPR + reduced-motion pattern)

import { useEffect, useMemo, type ReactNode } from "react";
import { ContourLine } from "@/components/recordings/ContourLine";
import { RangeReadout } from "@/components/recordings/RangeReadout";
import { VibratoReadout } from "@/components/recordings/VibratoReadout";
import { formatDurationShort, formatRelative } from "@/lib/duration-format";
import { formatNoteShort, hzToNote, midiToHz } from "@/lib/note-format";
import { useAnalysisStore, selectLatestContour } from "@/stores/analysisStore";
import { useRecordingsStore } from "@/stores/recordingsStore";

/** MIDI → "A4" / "C#5" formatter via the existing equal-tempered helper. */
function formatMidiNote(midi: number, a4Hz: number): string {
  const hz = midiToHz(midi, a4Hz);
  return formatNoteShort(hzToNote(hz, a4Hz));
}

function formatSignedCents(cents: number): string {
  const sign = cents >= 0 ? "+" : "";
  return `${sign}${cents.toFixed(1)}`;
}

function formatVoicedPercent(ratio: number): string {
  const pct = Math.round(ratio * 100);
  return `${pct}%`;
}

export function RecordingDetail(): ReactNode {
  const items = useRecordingsStore((s) => s.items);
  const currentRecordingId = useRecordingsStore((s) => s.currentRecordingId);

  const summary = useAnalysisStore((s) =>
    currentRecordingId !== null ? s.byRecording.get(currentRecordingId) : undefined,
  );
  const inProgress = useAnalysisStore((s) =>
    currentRecordingId !== null ? s.inProgress.has(currentRecordingId) : false,
  );
  const progressPercent = useAnalysisStore((s) =>
    currentRecordingId !== null ? s.progressByRecording.get(currentRecordingId) : undefined,
  );
  const errorMsg = useAnalysisStore((s) =>
    currentRecordingId !== null ? s.errors.get(currentRecordingId) : undefined,
  );
  const contour = useAnalysisStore((s) =>
    currentRecordingId !== null ? selectLatestContour(s, currentRecordingId) : undefined,
  );
  const analyze = useAnalysisStore((s) => s.analyze);

  // The recording row provides the header metadata; we look it up from the
  // already-loaded list rather than firing a per-row IPC.
  const recording = useMemo(() => {
    if (currentRecordingId === null) return undefined;
    return items.find((r) => r.id === currentRecordingId);
  }, [currentRecordingId, items]);

  // Trigger an analyze on selection. The cached path resolves <50ms and
  // populates `byRecording` synchronously from the IPC response; the
  // forced-refresh path is driven by the Re-analyze button below.
  useEffect(() => {
    if (currentRecordingId === null) return;
    void analyze(currentRecordingId);
  }, [currentRecordingId, analyze]);

  // Pin "now" once per selected-row change so the relative-time label is
  // stable across summary repaints.
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const now = useMemo(() => Date.now(), [currentRecordingId]);

  if (currentRecordingId === null || recording === undefined) return null;

  const displayLabel = recording.userLabel ?? recording.filename;
  const showProgress = inProgress;
  const percent =
    typeof progressPercent === "number" ? Math.max(0, Math.min(100, progressPercent)) : undefined;

  const onReanalyze = (): void => {
    void analyze(currentRecordingId, { forceRefresh: true });
  };

  const medianNoteLabel =
    summary !== undefined ? formatMidiNote(summary.medianMidi, recording.a4Hz) : "—";
  const medianCentsLabel = summary !== undefined ? formatSignedCents(summary.medianCents) : "—";
  const voicedPctLabel = summary !== undefined ? formatVoicedPercent(summary.voicedRatio) : "—";
  const cacheBadge = summary !== undefined ? (summary.wasCached ? "Cached" : "Fresh") : null;

  return (
    <section
      data-testid="recording-detail"
      aria-label="Recording detail"
      // aria-live=polite: when a row is selected the panel is appended below
      // the listbox in the same drawer; without a live region screen-reader
      // users get no signal that ~150 lines of new content (header, summary
      // card, contour figure, Re-analyze button) just appeared. Polite
      // (rather than assertive) so the announcement waits for ATs to finish
      // current speech.
      aria-live="polite"
      className="flex flex-col gap-3 rounded-md border border-slate-700 bg-slate-900/50 p-3"
    >
      <header
        data-testid="recording-detail-header"
        className="flex flex-col gap-1 border-b border-slate-700 pb-2"
      >
        <div className="flex items-center justify-between gap-2">
          <span
            className="truncate font-medium text-slate-100"
            title={displayLabel}
            data-testid="recording-detail-filename"
          >
            {displayLabel}
          </span>
          <span
            aria-label={`Instrument profile ${recording.instrumentProfile}`}
            className="shrink-0 rounded-full border border-slate-600 px-2 py-0.5 text-xs text-slate-300"
          >
            {recording.instrumentProfile}
          </span>
        </div>
        <div className="flex items-center justify-between gap-2 text-xs text-slate-400">
          <span>{formatRelative(recording.createdAt, now)}</span>
          <span aria-label={`Duration ${formatDurationShort(recording.durationMs)}`}>
            {formatDurationShort(recording.durationMs)}
          </span>
        </div>
        <div className="flex items-center gap-2 text-xs text-slate-300">
          <span
            data-testid="a4-pill"
            className="rounded-full border border-slate-600 px-2 py-0.5"
            aria-label={`Tuning reference A4 = ${recording.a4Hz} Hz`}
          >
            {`A4 = ${recording.a4Hz} Hz`}
          </span>
        </div>
      </header>

      <section
        role="group"
        aria-label="Analysis summary"
        aria-busy={showProgress}
        data-testid="analysis-summary"
        className="flex flex-col gap-2"
      >
        {errorMsg !== undefined && !showProgress ? (
          <div role="alert" className="text-xs text-rose-300">
            Analysis failed: {errorMsg}
          </div>
        ) : null}

        {showProgress ? (
          <div className="flex flex-col gap-1">
            <progress
              id="analysis-progress-bar"
              data-testid="analysis-progress"
              role="progressbar"
              aria-label="Analyzing recording"
              max={100}
              {...(percent !== undefined ? { value: percent } : {})}
              className="h-2 w-full overflow-hidden rounded-full bg-slate-800"
            />
            <span className="text-xs text-slate-400">Analyzing…</span>
          </div>
        ) : (
          <div className="flex flex-wrap items-center gap-x-4 gap-y-1 text-sm">
            <div className="flex flex-col">
              <span className="text-xs font-semibold uppercase tracking-wide text-slate-300">
                Median
              </span>
              <span className="font-medium text-slate-100" data-testid="summary-median-note">
                {medianNoteLabel}
              </span>
            </div>
            <div className="flex flex-col">
              <span className="text-xs font-semibold uppercase tracking-wide text-slate-300">
                Cents
              </span>
              <span className="font-mono text-slate-100" data-testid="summary-median-cents">
                {medianCentsLabel}
              </span>
            </div>
            <div className="flex flex-col">
              <span className="text-xs font-semibold uppercase tracking-wide text-slate-300">
                Voiced
              </span>
              <span className="font-mono text-slate-100" data-testid="summary-voiced">
                {voicedPctLabel}
              </span>
            </div>
            {cacheBadge !== null ? (
              <span
                data-testid="cache-badge"
                data-state={cacheBadge.toLowerCase()}
                className={[
                  "ml-auto rounded-full border px-2 py-0.5 text-xs",
                  cacheBadge === "Cached"
                    ? "border-cyan-700 text-cyan-300"
                    : "border-emerald-700 text-emerald-300",
                ].join(" ")}
              >
                {cacheBadge}
              </span>
            ) : null}
          </div>
        )}

        <div>
          <button
            type="button"
            data-testid="reanalyze"
            // Use aria-disabled instead of the native `disabled` attribute so
            // focus is not silently dropped to <body> when the button is
            // toggled into the busy state on Chromium / WebKit. The onClick
            // guard preserves the no-op semantics while keeping keyboard
            // focus on the control across the in-flight state. aria-busy
            // is mirrored on the parent <section role="group">; aria-
            // describedby points at the live progressbar so AT users hear
            // the percent advance instead of two unrelated announcements.
            aria-disabled={showProgress}
            aria-describedby={showProgress ? "analysis-progress-bar" : undefined}
            onClick={(e) => {
              if (showProgress) {
                e.preventDefault();
                return;
              }
              onReanalyze();
            }}
            className={[
              "rounded-md border border-slate-600 bg-slate-800 px-3 py-1 text-xs text-slate-200",
              "hover:bg-slate-700 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400",
              "aria-disabled:cursor-not-allowed aria-disabled:opacity-60",
            ].join(" ")}
          >
            Re-analyze
          </button>
        </div>
      </section>

      <div className="grid gap-3 md:grid-cols-2">
        <RangeReadout summary={summary} a4Hz={recording.a4Hz} />
        <VibratoReadout summary={summary} />
      </div>

      <ContourLine summary={summary} contour={contour} />
    </section>
  );
}
