// usePitchStream — Tauri Channel<PitchUpdate> wired into a hot-path ring.
//
// React component contract:
//   const { ringRef, retry } = usePitchStream();
//   useEffect(() => { rAF using ringRef.current.peekLatest(); ... }, []);
//
// The hook owns:
//   1. A `RingBuffer<PitchUpdate>` of capacity 256 (~2.7 s @ 93 Hz).
//   2. The Tauri Channel construction + `start_capture` invoke.
//   3. The `stop_capture` cleanup on unmount.
//   4. The mirror to `tunerStore` for slow facts (device name, capture flag,
//      last-voiced-note label for ARIA live regions).
//   5. A `retry()` action that re-issues `start_capture` with the cached
//      settings — used by `PermissionNotice` after the user has granted
//      microphone access.
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
//   docs/design/DESIGN.md §9.3 (audio backend errors / recovery)
//   tests/e2e/helpers/tauri-mock.ts (synthetic Channel)

import { useCallback, useEffect, useRef } from "react";
import { Channel, invoke } from "@tauri-apps/api/core";
import { RingBuffer } from "@/lib/ring";
import { hzToNote, formatNoteShort } from "@/lib/note-format";
import { registerTestListener } from "@/lib/test-hooks";
import { snapshotSettingsForIpc, useSettingsStore } from "@/stores/settingsStore";
import { useTunerStore } from "@/stores/tunerStore";
import type { PitchUpdate } from "@/types/pitch";
import type { AudioParams } from "@/types/settings";
import type { AudioBackendEvent } from "@/types/audio-event";

export const PITCH_RING_CAPACITY = 256;

/** Sentinel error string the Rust shell emits for `AudioError::PermissionDenied`.
 *  Routed to `setDeviceStatus("permission_denied")` rather than `setStartError`
 *  so the PermissionNotice banner can render. */
const PERMISSION_DENIED_SENTINEL = "permission_denied";

interface StartCaptureResponse {
  device_name?: string;
  deviceName?: string;
  sample_rate_hz?: number;
  sampleRateHz?: number;
  window_samples?: number;
  windowSamples?: number;
  hop_samples?: number;
  hopSamples?: number;
  channels?: number;
}

function normaliseStartResponse(raw: StartCaptureResponse | null | undefined): {
  deviceName: string;
  audioParams: AudioParams;
  channels: number;
} {
  const deviceName = raw?.device_name ?? raw?.deviceName ?? "default";
  const sampleRateHz = raw?.sample_rate_hz ?? raw?.sampleRateHz ?? 48000;
  const windowSamples = raw?.window_samples ?? raw?.windowSamples ?? 2048;
  const hopSamples = raw?.hop_samples ?? raw?.hopSamples ?? 512;
  const channels = raw?.channels ?? 1;
  return {
    deviceName,
    audioParams: { sampleRateHz, windowSamples, hopSamples },
    channels,
  };
}

function isPermissionDeniedError(err: unknown): boolean {
  const msg = err instanceof Error ? err.message : String(err);
  return msg.toLowerCase().includes(PERMISSION_DENIED_SENTINEL);
}

export interface UsePitchStreamApi {
  /** Stable ref to the live PitchUpdate ring. */
  ringRef: React.RefObject<RingBuffer<PitchUpdate>>;
  /** Re-issue start_capture after a permission flow. Resolves once the
   *  shell side either confirms or rejects again. */
  retry: () => Promise<void>;
}

/**
 * Mount-once hook returning a stable ref to the live PitchUpdate ring and a
 * retry action that can re-run start_capture after a permission_denied.
 */
export function usePitchStream(): UsePitchStreamApi {
  // Stable ref to the ring across re-renders.
  const ringRef = useRef<RingBuffer<PitchUpdate>>(new RingBuffer<PitchUpdate>(PITCH_RING_CAPACITY));
  // The Channel is recreated on every (re)start so the Rust side does not
  // hold a dangling callback id when we re-issue start_capture during retry.
  const channelRef = useRef<Channel<PitchUpdate> | null>(null);

  // Define the channel-message handler once and reuse for fresh Channels.
  const handleMessage = useCallback((payload: PitchUpdate): void => {
    if (!isValidPitchUpdate(payload)) {
      console.warn("usePitchStream: dropping malformed PitchUpdate", payload);
      return;
    }
    const update = payload;
    ringRef.current.push(update);
    if (update.voiced) {
      const a4 = useSettingsStore.getState().a4Hz;
      const note = hzToNote(update.f0_hz, a4);
      const label = formatNoteShort(note);
      const prev = useTunerStore.getState().lastVoicedNoteLabel;
      if (prev !== label) {
        useTunerStore.getState().setLastVoicedNoteLabel(label);
      }
    }
  }, []);

  const startCapture = useCallback(async (): Promise<void> => {
    const channel = new Channel<PitchUpdate>();
    channel.onmessage = handleMessage;
    channelRef.current = channel;
    // Out-of-band device events (Disconnected / Underrun / FormatChanged).
    // The Rust shell wires this into the cpal `err_fn`; Phase 1.3 surfaces
    // `Disconnected` as a toast in `tunerStore.setDeviceStatus("disconnected")`.
    const events = new Channel<AudioBackendEvent>();
    events.onmessage = (ev: AudioBackendEvent) => {
      switch (ev.kind) {
        case "disconnected":
          useTunerStore.getState().setDeviceStatus("disconnected");
          break;
        case "format_changed":
          // Mirror the renegotiated rate/channel count into the tuner store
          // so the UI's audio-format readout stays in sync.
          useTunerStore.getState().setNegotiatedFormat({
            rateHz: ev.new.sample_rate,
            channels: ev.new.channels,
          });
          break;
        case "underrun":
          // Underrun events are advisory; we log but do not surface UI.
          // The DSP worker emits its own structured tracing on advance.
          break;
        default:
          break;
      }
    };
    try {
      const raw = await invoke<StartCaptureResponse>("start_capture", {
        channel,
        events,
        settings: snapshotSettingsForIpc(useSettingsStore.getState()),
      });
      const { deviceName, audioParams, channels } = normaliseStartResponse(raw);
      useTunerStore.getState().setCaptureStarted(deviceName);
      useTunerStore.getState().setNegotiatedFormat({ rateHz: audioParams.sampleRateHz, channels });
      useSettingsStore.getState().setAudioParams(audioParams);
    } catch (err: unknown) {
      if (isPermissionDeniedError(err)) {
        useTunerStore.getState().setDeviceStatus("permission_denied");
        return;
      }
      const msg = err instanceof Error ? err.message : String(err);
      useTunerStore.getState().setStartError(msg);
    }
  }, [handleMessage]);

  useEffect(() => {
    let cancelled = false;
    const ring = ringRef.current;

    // Expose a raw push hook for the E2E mock harness.
    const unregisterTestListener = registerTestListener("pitch-update", (payload) => {
      handleMessage(payload as PitchUpdate);
    });

    void startCapture().then(() => {
      if (cancelled) {
        // The component unmounted during start_capture; the cleanup branch
        // below will idempotently stop the capture.
      }
    });

    return () => {
      cancelled = true;
      unregisterTestListener();
      void invoke("stop_capture").catch(() => {
        /* swallow: the shell handles repeated stops idempotently. */
      });
      useTunerStore.getState().setCaptureStopped();
      // Clear the ring on unmount so a remount (StrictMode double-invoke,
      // hot-reload, or future routing) does not surface a stale pre-stop
      // frame on the first rAF tick before new Channel data arrives.
      ring.clear();
    };
  }, [handleMessage, startCapture]);

  const retry = useCallback(async (): Promise<void> => {
    // Stop any in-flight capture before re-issuing start_capture. The Rust
    // side documents stop_capture as idempotent.
    try {
      await invoke("stop_capture");
    } catch {
      /* ignore */
    }
    useTunerStore.getState().clearDeviceError();
    await startCapture();
  }, [startCapture]);

  return { ringRef, retry };
}

function isValidPitchUpdate(p: unknown): p is PitchUpdate {
  if (p === null || typeof p !== "object") return false;
  const o = p as Record<string, unknown>;
  // Always-required fields. `target_hz` / `target_midi` are computed by the
  // Rust pipeline from `f0_hz`; for unvoiced frames the test harness can
  // send 0 / -Infinity. We only require *numeric* values there
  // (typeof === "number") and don't insist on `isFinite`.
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
