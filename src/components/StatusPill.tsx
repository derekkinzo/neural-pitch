// StatusPill — capture-state badge in the top-left of the tuner.
//
// Surfaces four pieces of slow-changing data:
//   - device name (or "—" before start_capture resolves)
//   - sample rate (Hz) once the audio params are known, suffixed with the
//     channel count and a `±` glyph if the negotiated rate differs from the
//     requested one)
//   - capture state (idle / live / error) with a small color dot
//   - active auto-prior range with two visual variants:
//       * Auto (instrumentHint === "Generic"): magic-wand glyph, sky border
//       * Explicit (any other hint): lock glyph, slate border
//
// All come from Zustand selectors, NOT the rAF ring. No animation.
//

import { useId, type ReactNode } from "react";
import { useSettingsStore } from "@/stores/settingsStore";
import { useTunerStore } from "@/stores/tunerStore";
import { FALLBACK_GENERIC, INSTRUMENT_RANGE_TABLE, type InstrumentHint } from "@/types/settings";

function formatRange(range: readonly [number, number]): string {
  const [lo, hi] = range;
  return `${Math.round(lo)}–${Math.round(hi)} Hz`;
}

function formatRateKHz(hz: number): string {
  // Drop the trailing decimal when the value is an integer multiple of 1000
  // (48 kHz reads "48 kHz", 44100 reads "44.1 kHz").
  const k = hz / 1000;
  return Number.isInteger(k) ? `${k} kHz` : `${k.toFixed(1)} kHz`;
}

interface PriorBadgeContent {
  readonly mode: "auto" | "explicit";
  readonly text: string;
  readonly icon: string;
  readonly className: string;
}

function priorBadge(
  hint: InstrumentHint,
  priorRange: readonly [number, number] | null,
): PriorBadgeContent {
  if (hint === "Generic") {
    const range = priorRange ?? FALLBACK_GENERIC;
    return {
      mode: "auto",
      text: `Auto · ${formatRange(range)}`,
      icon: "✨",
      className: "border-sky-500/40 text-sky-200",
    };
  }
  const range = INSTRUMENT_RANGE_TABLE[hint];
  return {
    mode: "explicit",
    text: `${hint} ${formatRange(range)}`,
    icon: "\u{1F512}",
    className: "border-slate-600 text-slate-200",
  };
}

export function StatusPill(): ReactNode {
  const isCapturing = useTunerStore((s) => s.isCapturing);
  const deviceName = useTunerStore((s) => s.deviceName);
  const startError = useTunerStore((s) => s.startError);
  const priorRange = useTunerStore((s) => s.priorRange);
  const negotiatedRateHz = useTunerStore((s) => s.negotiatedRateHz);
  const negotiatedChannels = useTunerStore((s) => s.negotiatedChannels);
  const requestedRateHz = useTunerStore((s) => s.requestedRateHz);
  const audioParams = useSettingsStore((s) => s.audioParams);
  const instrumentHint = useSettingsStore((s) => s.instrumentHint);

  // useId() instead of a module-level constant so two StatusPill instances
  // (HMR overlap, future split view, Storybook, side-by-side tests) cannot
  // collide on the aria-describedby anchor.
  const rateMismatchHelpId = useId();

  const dotClass =
    startError !== null ? "bg-rose-400" : isCapturing ? "bg-emerald-400" : "bg-slate-500";

  const stateLabel = startError !== null ? "error" : isCapturing ? "live" : "idle";

  // Choose the display rate: prefer the explicit negotiated value, fall
  // back to whatever audioParams reports (the rate the start_capture
  // response carried), else the placeholder "—".
  const displayRateHz = negotiatedRateHz ?? audioParams?.sampleRateHz ?? null;
  const channels = negotiatedChannels ?? 1;
  const channelText = channels === 1 ? "mono" : `${channels}ch`;
  const mismatch = negotiatedRateHz !== null && negotiatedRateHz !== requestedRateHz;
  const rateBaseText = displayRateHz !== null ? formatRateKHz(displayRateHz) : "—";
  const rateText = displayRateHz !== null ? `${rateBaseText} · ${channelText}` : rateBaseText;
  const rateDisplayText = mismatch ? `${rateText} ±` : rateText;

  const badge = priorBadge(instrumentHint, priorRange);

  // Interpolate the actual negotiated rate into the rate-mismatch help so
  // the described-by text matches what the user sees in the readout
  // (e.g. "48 kHz" not the previously hard-coded "44.1 kHz").
  const rateMismatchHelpText =
    displayRateHz !== null
      ? `Negotiated ${formatRateKHz(displayRateHz)}; engine resamples on demand.`
      : "Negotiated rate differs from the requested rate; engine resamples on demand.";

  return (
    <div
      data-testid="status-pill"
      data-state={stateLabel}
      className="inline-flex items-center gap-2 rounded-full border border-slate-700 bg-slate-900/60 px-3 py-1 text-xs text-slate-300"
    >
      <span aria-hidden="true" className={`inline-block h-2 w-2 rounded-full ${dotClass}`} />
      <span className="font-medium uppercase tracking-wide text-slate-200">{stateLabel}</span>
      <span aria-hidden="true" className="text-slate-600">
        ·
      </span>
      <span data-testid="status-device">{deviceName ?? "—"}</span>
      <span aria-hidden="true" className="text-slate-600">
        ·
      </span>
      <span
        data-testid="status-rate"
        {...(mismatch
          ? {
              title: rateMismatchHelpText,
              "aria-describedby": rateMismatchHelpId,
            }
          : {})}
      >
        {rateDisplayText}
      </span>
      <span aria-hidden="true" className="text-slate-600">
        ·
      </span>
      <span
        data-testid="status-prior"
        data-prior-mode={badge.mode}
        className={`inline-flex items-center gap-1 rounded-full border px-2 py-0.5 ${badge.className}`}
      >
        {/* Decorative icon: the visible text "Auto · 80–620 Hz" / "Guitar
            80–1300 Hz" plus data-prior-mode already convey the badge
            meaning, so the glyph is redundant for AT. aria-hidden avoids
            "image auto-prior Auto middle dot 80 to 620 Hz"-style
            duplicate announcements on NVDA / JAWS. */}
        <span aria-hidden="true">{badge.icon}</span>
        <span>{badge.text}</span>
      </span>
      {mismatch ? (
        <span id={rateMismatchHelpId} className="sr-only">
          {rateMismatchHelpText}
        </span>
      ) : null}
    </div>
  );
}
