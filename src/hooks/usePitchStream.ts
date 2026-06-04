// usePitchStream — Tauri Channel<PitchUpdate> wired into a hot-path ring.
//
// React component contract:
//   const ring = usePitchStream();             // ref to RingBuffer<PitchUpdate>
//   useEffect(() => { rAF using ring.peekLatest(); ... }, []);
//
// The hook owns:
//   1. A `RingBuffer<PitchUpdate>` of capacity 256 (~2.7 s @ 93 Hz).
//   2. The Tauri Channel construction + `start_capture` invoke.
//   3. The `stop_capture` cleanup on unmount.
//   4. The mirror to `tunerStore` for slow facts (device name, capture flag,
//      last-voiced-note label for ARIA live regions).
//
// Per ADR-0003 the rAF readers in `CentsMeter` / `HistoryStrip` read the ring
// directly — no setState on per-frame updates.
//
// Test surface: when running under the Tier-5 E2E mock, the helper exposes
// `window.__neuralPitchTestHooks.pushPitchUpdate(frame)` which routes through
// the same Channel callback wiring.
//
// Cross-references:
//   docs/design/DESIGN.md §7 (Tauri channel wiring)
//   tests/e2e/helpers/tauri-mock.ts (synthetic Channel)

import { useEffect, useRef } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { RingBuffer } from "@/lib/ring";
import { hzToNote, formatNoteShort } from "@/lib/note-format";
import { snapshotSettingsForIpc, useSettingsStore } from "@/stores/settingsStore";
import { useTunerStore } from "@/stores/tunerStore";
import type { PitchUpdate } from "@/types/pitch";
import type { AudioParams } from "@/types/settings";

export const PITCH_RING_CAPACITY = 256;

interface StartCaptureResponse {
  device_name?: string;
  deviceName?: string;
  sample_rate_hz?: number;
  sampleRateHz?: number;
  window_samples?: number;
  windowSamples?: number;
  hop_samples?: number;
  hopSamples?: number;
}

function normaliseStartResponse(raw: StartCaptureResponse | null | undefined): {
  deviceName: string;
  audioParams: AudioParams;
} {
  const deviceName = raw?.device_name ?? raw?.deviceName ?? "default";
  const sampleRateHz = raw?.sample_rate_hz ?? raw?.sampleRateHz ?? 48000;
  const windowSamples = raw?.window_samples ?? raw?.windowSamples ?? 2048;
  const hopSamples = raw?.hop_samples ?? raw?.hopSamples ?? 512;
  return {
    deviceName,
    audioParams: { sampleRateHz, windowSamples, hopSamples },
  };
}

/**
 * Mount-once hook returning a stable ref to the live PitchUpdate ring.
 * Components that paint on rAF read the ring through the returned ref.
 */
export function usePitchStream(): React.RefObject<RingBuffer<PitchUpdate>> {
  // Stable ref to the ring across re-renders.
  const ringRef = useRef<RingBuffer<PitchUpdate>>(new RingBuffer<PitchUpdate>(PITCH_RING_CAPACITY));

  useEffect(() => {
    let cancelled = false;
    const ring = ringRef.current;

    const channel = new Channel<PitchUpdate>();
    channel.onmessage = (payload: PitchUpdate) => {
      // Defensive validation — a malformed PitchUpdate from Rust (or a
      // broken test harness) would otherwise throw inside the rAF readers
      // (NaN clamps, undefined.f0_hz). We log+drop the frame instead.
      if (!isValidPitchUpdate(payload)) {
        console.warn("usePitchStream: dropping malformed PitchUpdate", payload);
        return;
      }
      const update = payload;
      ring.push(update);
      // Mirror the voiced-note label into the slow store. We use `f0_hz`
      // (the measured fundamental) rather than `target_hz` (the
      // equal-tempered nearest note) because `NoteDisplay` renders the
      // glyph from `f0_hz`; AT-announced label and visible note must
      // agree near the cents-50 boundary.
      if (update.voiced) {
        const a4 = useSettingsStore.getState().a4Hz;
        const note = hzToNote(update.f0_hz, a4);
        const label = formatNoteShort(note);
        const prev = useTunerStore.getState().lastVoicedNoteLabel;
        if (prev !== label) {
          useTunerStore.getState().setLastVoicedNoteLabel(label);
        }
      }
    };

    function isValidPitchUpdate(p: unknown): p is PitchUpdate {
      if (p === null || typeof p !== "object") return false;
      const o = p as Record<string, unknown>;
      // Always-required fields. `target_hz` / `target_midi` are computed
      // by the Rust pipeline from `f0_hz`; for unvoiced frames the test
      // harness can send 0 / -Infinity. We only require *numeric* values
      // there (typeof === "number") and don't insist on `isFinite`.
      const numeric = (k: string): boolean => typeof o[k] === "number";
      return (
        typeof o["voiced"] === "boolean" &&
        Number.isFinite(o["f0_hz"]) &&
        Number.isFinite(o["smoothed_cents"]) &&
        Number.isFinite(o["confidence"]) &&
        Number.isFinite(o["timestamp_samples"]) &&
        numeric("target_hz") &&
        numeric("target_midi")
      );
    }

    // Expose a raw push hook for the E2E mock harness. In production this is
    // a no-op because `__neuralPitchTestHooks` is only populated by the
    // Playwright init script.
    type WindowWithHooks = Window & {
      __neuralPitchTestHooks?: {
        listeners: Map<string, Array<(payload: unknown) => void>>;
      };
    };
    const w = window as WindowWithHooks;
    const hooks = w.__neuralPitchTestHooks;
    let unregisterTestListener: (() => void) | null = null;
    if (hooks !== undefined) {
      const fn = (payload: unknown): void => {
        // The test harness sends payloads in the same snake_case shape as the
        // Rust side; trust the spec author to provide the right shape.
        channel.onmessage(payload as PitchUpdate);
      };
      const list = hooks.listeners.get("pitch-update") ?? [];
      list.push(fn);
      hooks.listeners.set("pitch-update", list);
      unregisterTestListener = () => {
        const cur = hooks.listeners.get("pitch-update") ?? [];
        hooks.listeners.set(
          "pitch-update",
          cur.filter((f) => f !== fn),
        );
      };
    }

    void invoke<StartCaptureResponse>("start_capture", {
      channel,
      settings: snapshotSettingsForIpc(useSettingsStore.getState()),
    })
      .then((raw) => {
        if (cancelled) return;
        const { deviceName, audioParams } = normaliseStartResponse(raw);
        useTunerStore.getState().setCaptureStarted(deviceName);
        useSettingsStore.getState().setAudioParams(audioParams);
      })
      .catch((err: unknown) => {
        if (cancelled) return;
        const msg = err instanceof Error ? err.message : String(err);
        useTunerStore.getState().setStartError(msg);
      });

    return () => {
      cancelled = true;
      unregisterTestListener?.();
      void invoke("stop_capture").catch(() => {
        /* swallow: the shell handles repeated stops idempotently. */
      });
      useTunerStore.getState().setCaptureStopped();
      // Clear the ring on unmount so a remount (StrictMode double-invoke,
      // hot-reload, or future routing) does not surface a stale pre-stop
      // frame on the first rAF tick before new Channel data arrives.
      ring.clear();
    };
  }, []);

  return ringRef;
}
