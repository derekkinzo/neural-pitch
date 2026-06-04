// StatusPill — capture-state badge in the top-left of the tuner.
//
// Surfaces three pieces of slow-changing data:
//   - device name (or "—" before start_capture resolves)
//   - sample rate (Hz) once the audio params are known
//   - capture state (idle / live / error) with a small color dot
//
// All three come from Zustand selectors, NOT the rAF ring. No animation.
//
// Cross-references:
//   docs/design/DESIGN.md §1 (header row)

import { type ReactNode } from "react";
import { useSettingsStore } from "@/stores/settingsStore";
import { useTunerStore } from "@/stores/tunerStore";

export function StatusPill(): ReactNode {
  const isCapturing = useTunerStore((s) => s.isCapturing);
  const deviceName = useTunerStore((s) => s.deviceName);
  const startError = useTunerStore((s) => s.startError);
  const audioParams = useSettingsStore((s) => s.audioParams);

  const dotClass =
    startError !== null ? "bg-rose-400" : isCapturing ? "bg-emerald-400" : "bg-slate-500";

  const stateLabel = startError !== null ? "error" : isCapturing ? "live" : "idle";
  const sampleRateText =
    audioParams !== null ? `${(audioParams.sampleRateHz / 1000).toFixed(1)} kHz` : "—";

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
      <span data-testid="status-rate">{sampleRateText}</span>
    </div>
  );
}
