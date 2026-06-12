// SettingsDrawer — right-side sheet with A4, instrument hint, smoothing
// slider, and read-only audio-params output.
//
// Reads from `useSettingsStore` and writes via `useSettings` (debounced
// `configure` IPC). The drawer itself is a thin wrapper around the vendored
// `Drawer` primitive in `components/ui/`.
//

import { useEffect, useId, useState, type ChangeEvent, type ReactNode } from "react";
import { Drawer } from "@/components/ui/Drawer";
import { Select } from "@/components/ui/Select";
import { Slider } from "@/components/ui/Slider";
import { useSettings } from "@/hooks/useSettings";
import { useSettingsStore } from "@/stores/settingsStore";
import {
  A4_MAX_HZ,
  A4_MIN_HZ,
  A4_PRESETS,
  INSTRUMENT_HINTS,
  NOTE_LABEL_MODES,
  SMOOTHING_MAX_MS,
  SMOOTHING_MIN_MS,
  SMOOTHING_STEP_MS,
  type InstrumentHint,
  type NoteLabelMode,
} from "@/types/settings";

export interface SettingsDrawerProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

export function SettingsDrawer({ open, onOpenChange }: SettingsDrawerProps): ReactNode {
  const a4Hz = useSettingsStore((s) => s.a4Hz);
  const instrumentHint = useSettingsStore((s) => s.instrumentHint);
  const smoothingMs = useSettingsStore((s) => s.smoothingMs);
  const noteLabelMode = useSettingsStore((s) => s.noteLabelMode);
  const setNoteLabelMode = useSettingsStore((s) => s.setNoteLabelMode);
  const audioParams = useSettingsStore((s) => s.audioParams);

  const { setA4Hz, setInstrumentHint, setSmoothingMs } = useSettings();

  const a4InputId = useId();
  const a4PresetId = useId();
  const instrumentId = useId();
  const smoothingId = useId();
  const noteLabelId = useId();

  // Local input state keeps numeric typing snappy without spamming the store.
  const [a4Draft, setA4Draft] = useState<string>(String(a4Hz));
  // Reset draft when the store value changes externally (e.g. preset click)
  // and the input is not currently focused. Compare to the React-assigned
  // input id so the focus guard does not depend on the test-id attribute.
  useEffect(() => {
    const active = document.activeElement as HTMLElement | null;
    if (active?.id === a4InputId) return;
    setA4Draft(String(a4Hz));
  }, [a4Hz, a4InputId]);

  const onA4Input = (e: ChangeEvent<HTMLInputElement>): void => {
    const raw = e.currentTarget.value;
    setA4Draft(raw);
    const n = Number(raw);
    if (Number.isFinite(n)) setA4Hz(n);
  };

  const onA4Preset = (n: number): void => {
    setA4Draft(String(n));
    setA4Hz(n);
  };

  return (
    <Drawer
      open={open}
      onOpenChange={onOpenChange}
      title="Tuner settings"
      closeLabel="Close settings"
    >
      <div className="flex flex-col gap-6 text-sm text-slate-200">
        <section className="flex flex-col gap-2">
          <label htmlFor={a4PresetId} className="font-medium">
            A4 reference
          </label>
          <Select
            id={a4PresetId}
            aria-label="A4 reference preset"
            value={A4_PRESETS.includes(a4Hz) ? a4Hz : 440}
            onValueChange={onA4Preset}
            numeric
          >
            {A4_PRESETS.map((p) => (
              <option key={p} value={p}>
                {p} Hz
              </option>
            ))}
          </Select>
          <input
            id={a4InputId}
            data-testid="a4-input"
            aria-label="A4 reference frequency in Hertz"
            type="number"
            min={A4_MIN_HZ}
            max={A4_MAX_HZ}
            step={0.1}
            value={a4Draft}
            onChange={onA4Input}
            className="w-full rounded-md border border-slate-700 bg-slate-900 px-3 py-2 text-sm text-slate-100 shadow-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyan-400"
          />
        </section>

        <section className="flex flex-col gap-2">
          <label htmlFor={instrumentId} className="font-medium">
            Instrument hint
          </label>
          <Select
            id={instrumentId}
            value={instrumentHint}
            onValueChange={(v: string) => setInstrumentHint(v as InstrumentHint)}
          >
            {INSTRUMENT_HINTS.map((h) => (
              <option key={h} value={h}>
                {h}
              </option>
            ))}
          </Select>
        </section>

        <section className="flex flex-col gap-2">
          <label htmlFor={noteLabelId} className="font-medium">
            Note labels
          </label>
          <Select
            id={noteLabelId}
            value={noteLabelMode}
            onValueChange={(v: string) => setNoteLabelMode(v as NoteLabelMode)}
          >
            {NOTE_LABEL_MODES.map((m) => (
              <option key={m} value={m}>
                {m === "letter"
                  ? "Letter (C, D, E)"
                  : m === "movable-do"
                    ? "Movable do"
                    : "Fixed do"}
              </option>
            ))}
          </Select>
        </section>

        <section className="flex flex-col gap-2">
          <label htmlFor={smoothingId} className="font-medium">
            Smoothing window: <span data-testid="smoothing-readout">{smoothingMs} ms</span>
          </label>
          <Slider
            id={smoothingId}
            data-testid="smoothing-slider"
            value={smoothingMs}
            min={SMOOTHING_MIN_MS}
            max={SMOOTHING_MAX_MS}
            step={SMOOTHING_STEP_MS}
            onValueChange={setSmoothingMs}
          />
        </section>

        <section className="flex flex-col gap-1 text-xs text-slate-400">
          <div className="font-medium uppercase tracking-wide text-slate-300">Audio parameters</div>
          <div className="flex justify-between">
            <span>Sample rate</span>
            <output data-testid="audio-sample-rate">
              {audioParams !== null ? `${audioParams.sampleRateHz} Hz` : "—"}
            </output>
          </div>
          <div className="flex justify-between">
            <span>Window</span>
            <output data-testid="audio-window">
              {audioParams !== null ? `${audioParams.windowSamples} samples` : "—"}
            </output>
          </div>
          <div className="flex justify-between">
            <span>Hop</span>
            <output data-testid="audio-hop">
              {audioParams !== null ? `${audioParams.hopSamples} samples` : "—"}
            </output>
          </div>
        </section>
      </div>
    </Drawer>
  );
}
