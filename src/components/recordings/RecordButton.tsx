// RecordButton — circular record control rendered in the tuner header.
//
// Three intrinsic states are derived from `useRecordingsStore`:
//   - `idle`       filled red dot
//   - `recording`  pulsing red dot (CSS keyframe; reduced-motion swaps to
//                  a static dot + the visible elapsed counter as the cue)
//   - `saving`     spinner glyph, button disabled until `stop_recording`
//                  resolves on the Rust side
//
// Click handling is driven by the store actions; the button does not
// invoke Tauri commands directly so the IPC surface stays in one place.
//
// Accessibility:
//   - `role="button"` is implicit on `<button>`; we still set
//     `aria-pressed` so AT users can discriminate the recording state
//     without relying on the visual pulse.
//   - `aria-label` regenerates each progress tick to read
//     "Stop recording (1:23)" while recording — the pre-existing
//     `aria-live` region from the tuner is NOT reused (label-only updates
//     are silent unless the user explicitly re-focuses the button).
//
// Cross-references:
//   docs/design/DESIGN.md §7.5 (RecordButton — top-right of header)
//   docs/adr/0011-recording-defaults-48k-24bit-mono-flac.md

import { type ReactNode } from "react";
import { formatElapsed } from "@/lib/duration-format";
import { useRecordingsStore } from "@/stores/recordingsStore";
import { useSettingsStore } from "@/stores/settingsStore";

type ButtonState = "idle" | "recording" | "saving";

function pickState(isRecording: boolean, saving: boolean, starting: boolean): ButtonState {
  // `starting` and `saving` both surface the same visual affordance —
  // a spinner-style busy state — because both represent an in-flight
  // IPC where the button must NOT be re-clicked. The store gates Stop
  // on `!starting` so the user cannot Stop a take that has not yet
  // started.
  if (saving || starting) return "saving";
  if (isRecording) return "recording";
  return "idle";
}

export function RecordButton(): ReactNode {
  const isRecording = useRecordingsStore((s) => s.isRecording);
  const saving = useRecordingsStore((s) => s.saving);
  const starting = useRecordingsStore((s) => s.starting);
  const elapsedMs = useRecordingsStore((s) => s.elapsedMs);
  const startRecording = useRecordingsStore((s) => s.startRecording);
  const stopRecording = useRecordingsStore((s) => s.stopRecording);
  const instrumentHint = useSettingsStore((s) => s.instrumentHint);

  const state = pickState(isRecording, saving, starting);
  const elapsedLabel = formatElapsed(elapsedMs);

  const ariaLabel =
    state === "recording"
      ? `Stop recording (${elapsedLabel})`
      : state === "saving"
        ? "Saving recording"
        : "Start recording";

  const onClick = (): void => {
    if (state === "idle") {
      void startRecording(instrumentHint);
    } else if (state === "recording") {
      void stopRecording();
    }
  };

  // Tailwind utility shorthand — small (32px) circular button next to the
  // gear, switching colour by state. Pulse is provided by the inner span
  // with the `record-pulse` class; the @keyframes lives in `index.css`
  // alongside the `prefers-reduced-motion` override.
  const baseBtn =
    "relative inline-flex h-8 w-8 items-center justify-center rounded-full border " +
    "transition-colors focus-visible:outline-none focus-visible:ring-2 " +
    "focus-visible:ring-rose-300 focus-visible:ring-offset-2 " +
    "focus-visible:ring-offset-slate-950 disabled:cursor-not-allowed " +
    "disabled:opacity-60";
  // The "recording" state adds a thicker outline (`ring-2`) so users
  // in forced-colors / Windows HCM keep a non-color cue when the rose
  // hue collapses to system colours. The pulse animation handles the
  // motion-friendly visual; reduced-motion users get the visible
  // elapsed counter; HCM users get the ring.
  const stateBtn =
    state === "recording"
      ? "border-rose-400/60 bg-rose-500/20 ring-2 ring-rose-400/80 hover:bg-rose-500/30"
      : state === "saving"
        ? "border-slate-600 bg-slate-800 text-slate-200"
        : "border-rose-500/40 bg-slate-900/60 hover:bg-rose-500/10";

  return (
    <div className="flex items-center gap-2">
      {/* Elapsed mm:ss counter — only rendered while recording or saving so
          (a) the idle layout stays compact and (b) we don't ship a low-contrast
          slate-600 token to the axe scan. The reduced-motion path described
          in the brief swaps the visual cue, but the counter only exists while
          a take is active either way. The reduced-motion spec asserts the
          counter ticks AFTER pressing record, so this gating is consistent
          with the test contract. */}
      {state !== "idle" ? (
        <span
          data-testid="record-elapsed"
          aria-hidden="false"
          className="select-none font-mono text-xs text-rose-200"
        >
          {elapsedLabel}
        </span>
      ) : null}
      <button
        type="button"
        data-testid="record-button"
        data-state={state}
        aria-pressed={isRecording}
        aria-label={ariaLabel}
        disabled={state === "saving"}
        onClick={onClick}
        className={[baseBtn, stateBtn].join(" ")}
      >
        {state === "saving" ? (
          <span aria-hidden="true" className="text-xs">
            …
          </span>
        ) : (
          // The pulse element is the visual cue while recording; `data-testid`
          // is referenced by the reduced-motion spec to assert the CSS
          // animation is suppressed. We render it in `idle` too (as a static
          // dot) so the layout stays stable across state transitions.
          <span
            data-testid="record-pulse"
            aria-hidden="true"
            className={[
              "inline-block h-4 w-4 rounded-full bg-rose-500",
              state === "recording" ? "record-pulse" : "",
            ]
              .join(" ")
              .trim()}
          />
        )}
      </button>
    </div>
  );
}
