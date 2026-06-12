// RecordingsList — drawer-hosted list of persisted takes.
//
// Reuses the existing `<Drawer>` primitive (right-side sheet, focus trap,
// Escape-to-close). Each row renders filename, relative createdAt time,
// formatted duration, instrument-profile badge, and a play button (which
// also serves as the row's selection trigger). The whole row participates
// in a WAI-ARIA APG listbox pattern so keyboard users can navigate with
// arrow keys instead of Tabbing through every row.
//
// Accessibility:
//   - `<ul role="listbox">` wraps the rows.
//   - Each `<li role="option">` carries `aria-selected` and a non-color
//     selection affordance (a leading "▸" glyph) — a border-color-only
//     cue would violate WCAG 1.4.1 in high-contrast / forced-colors
//     mode where the cyan border collapses.
//   - Roving tabindex: only the focused / selected row has `tabindex=0`;
//     the others are `tabindex=-1`. ArrowUp / ArrowDown / Home / End move
//     focus AND selection in lock-step.
//
// The store selector returns items already sorted descending by `createdAt`,
// so the rendered order does not need re-sorting per render.
//

import { useEffect, useMemo, useRef, type KeyboardEvent, type ReactNode } from "react";
import { Drawer } from "@/components/ui/Drawer";
import { PlaybackPanel } from "@/components/recordings/PlaybackPanel";
import { RecordingDetail } from "@/components/recordings/RecordingDetail";
import { formatDurationShort, formatRelative } from "@/lib/duration-format";
import { useAnalysisProgress } from "@/hooks/useAnalysisProgress";
import { useStemProgressSubscription } from "@/hooks/useStemProgressSubscription";
import { useTranscribeProgress } from "@/hooks/useTranscribeProgress";
import { useRecordingsStore } from "@/stores/recordingsStore";
import type { Recording } from "@/types/recording";

export interface RecordingsListProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function RecordingsList({ open, onOpenChange }: RecordingsListProps): ReactNode {
  const items = useRecordingsStore((s) => s.items);
  const refresh = useRecordingsStore((s) => s.refresh);
  const select = useRecordingsStore((s) => s.select);
  const currentRecordingId = useRecordingsStore((s) => s.currentRecordingId);

  // Subscribe to the `analysis-progress` event channel for the lifetime of
  // the drawer mount so the AnalysisSummary card's progress bar reflects
  // the live percent driven by the Rust analyzer.
  useAnalysisProgress();
  // Same wiring for the transcription channel — TranscribePanel
  // mounts inside RecordingDetail (a child of this drawer) so a single
  // subscription at the drawer root covers every panel instance.
  useTranscribeProgress();
  // Stems channel — StemSeparationPanel mounts inside the same
  // RecordingDetail subtree, so co-locating the subscription here keeps
  // a single fanout site for all per-recording IPC channels.
  useStemProgressSubscription();

  // Refresh on open so the list reflects any rows persisted while the
  // drawer was closed. Fire-and-forget; the action handles its own errors.
  useEffect(() => {
    if (!open) return;
    void refresh();
  }, [open, refresh]);

  // Pin "now" once per drawer-open — relative-time labels stay stable
  // across selection changes within a single open. Re-keying on `open`
  // (rather than `items`) keeps the label stable as the list grows
  // mid-session, matching the design intent. The eslint-disable below
  // is intentional: the dep is `open` precisely so the memo recomputes
  // when the drawer opens, but the rule wants `items` (which we DON'T
  // want to depend on).
  // eslint-disable-next-line react-hooks/exhaustive-deps
  const now = useMemo(() => Date.now(), [open]);

  const isEmpty = items.length === 0;

  // Compute the index of the currently selected row, or 0 if none. The
  // listbox roving-tabindex makes exactly one row keyboard-reachable;
  // all others have tabindex=-1.
  const selectedIndex = Math.max(
    0,
    items.findIndex((rec) => rec.id === currentRecordingId),
  );

  return (
    <Drawer
      open={open}
      onOpenChange={onOpenChange}
      title="Recordings"
      modal={false}
      closeLabel="Close recordings"
    >
      <div className="flex flex-col gap-4 text-sm text-slate-200">
        {/* The list anchor is always rendered (even empty) so specs that
            target `[data-testid=recordings-list] li` resolve consistently. */}
        <ul
          data-testid="recordings-list"
          aria-label="Saved recordings"
          role="listbox"
          // The empty list still needs a non-zero bounding box so Playwright's
          // `toBeVisible()` resolves while the empty-state status region
          // co-exists below (specs assert visibility in both populated +
          // empty cases).
          className={isEmpty ? "m-0 block min-h-[1px] list-none p-0" : "flex flex-col gap-2"}
        >
          {items.map((rec, idx) => (
            <RecordingRow
              key={rec.id}
              recording={rec}
              now={now}
              isSelected={rec.id === currentRecordingId}
              isFocusable={idx === selectedIndex}
              onSelect={() => select(rec.id)}
              onArrowMove={(dir) => {
                const next =
                  dir === "down"
                    ? Math.min(items.length - 1, selectedIndex + 1)
                    : dir === "up"
                      ? Math.max(0, selectedIndex - 1)
                      : dir === "home"
                        ? 0
                        : items.length - 1;
                const nextRec = items[next];
                if (nextRec !== undefined) select(nextRec.id);
              }}
            />
          ))}
        </ul>

        {isEmpty ? (
          <div
            role="status"
            data-testid="recordings-empty"
            className="rounded-md border border-slate-700 bg-slate-900/50 px-3 py-4 text-center text-slate-400"
          >
            No recordings yet — press the red dot to start your first one.
          </div>
        ) : null}

        <div data-testid="recording-detail-host" className="flex flex-col gap-3">
          <RecordingDetail />
          <PlaybackPanel />
        </div>
      </div>
    </Drawer>
  );
}

interface RowProps {
  recording: Recording;
  now: number;
  isSelected: boolean;
  isFocusable: boolean;
  onSelect: () => void;
  onArrowMove: (dir: "up" | "down" | "home" | "end") => void;
}

function RecordingRow({
  recording,
  now,
  isSelected,
  isFocusable,
  onSelect,
  onArrowMove,
}: RowProps): ReactNode {
  const displayLabel = recording.userLabel ?? recording.filename;
  const liRef = useRef<HTMLLIElement | null>(null);

  // When this row becomes the focusable one (selection moved to us via
  // keyboard), pull DOM focus so screen readers track the selection.
  useEffect(() => {
    if (isFocusable && isSelected && liRef.current !== null) {
      // Only steal focus if the active element is already inside the
      // listbox; do NOT yank focus from arbitrary surfaces.
      const active = document.activeElement;
      const list = liRef.current.closest('[role="listbox"]');
      if (list !== null && active instanceof Node && list.contains(active)) {
        liRef.current.focus();
      }
    }
  }, [isFocusable, isSelected]);

  const handleKeyDown = (e: KeyboardEvent<HTMLLIElement>): void => {
    switch (e.key) {
      case "ArrowDown":
        e.preventDefault();
        onArrowMove("down");
        break;
      case "ArrowUp":
        e.preventDefault();
        onArrowMove("up");
        break;
      case "Home":
        e.preventDefault();
        onArrowMove("home");
        break;
      case "End":
        e.preventDefault();
        onArrowMove("end");
        break;
      case "Enter":
      case " ":
        e.preventDefault();
        onSelect();
        break;
      default:
        break;
    }
  };

  return (
    <li
      ref={liRef}
      data-testid="recording-row"
      data-recording-id={recording.id}
      data-selected={isSelected ? "true" : "false"}
      role="option"
      aria-selected={isSelected}
      tabIndex={isFocusable ? 0 : -1}
      onKeyDown={handleKeyDown}
      onClick={onSelect}
      className={[
        "flex flex-col gap-1 rounded-md border px-3 py-2 outline-none",
        "focus-visible:ring-2 focus-visible:ring-cyan-400",
        isSelected ? "border-cyan-500/60 bg-slate-800" : "border-slate-700 bg-slate-900/40",
      ].join(" ")}
    >
      <div className="flex items-center justify-between gap-2">
        <span className="flex min-w-0 items-center gap-1.5 truncate font-medium text-slate-100">
          {/* Non-color selection affordance — a leading marker so users in
              forced-colors / Windows HCM still see which row is selected
              when the cyan border is suppressed. The marker is wrapped in
              an aria-hidden span because `aria-selected` already carries
              the semantic state. */}
          <span
            aria-hidden="true"
            data-testid="recording-row-marker"
            className={[
              "inline-block w-3 shrink-0 text-cyan-300",
              isSelected ? "" : "invisible",
            ].join(" ")}
          >
            {"▸"}
          </span>
          <span className="truncate" title={displayLabel}>
            {displayLabel}
          </span>
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
      <div className="mt-1 flex items-center gap-2 text-xs text-slate-300">
        {/* The row itself is the play / select control (`role=option`),
            and WAI-ARIA forbids interactive controls inside listbox
            options. Selecting a row mounts the PlaybackPanel below the
            list, which surfaces the actual `<audio controls>` element
            with play/pause/seek built in; the row only carries a
            non-interactive textual cue. */}
        <span data-testid="recording-play" aria-hidden="true" className="select-none">
          {isSelected ? "Now playing" : "Click to load"}
        </span>
      </div>
    </li>
  );
}
