// useDeviceEvents — single-subscription hook for the `audio:backend` Tauri
// event channel. Mirrors the Rust-side audio-backend bus enum into the
// slow Zustand store at <=1 Hz.
//
// Distinct from `types/audio-event.ts::AudioBackendEvent`: that type
// describes the out-of-band cpal `err_fn` Channel payloads
// (`disconnected | format_changed | underrun`); this one is the Tauri
// event-bus variant (`PriorNarrowed | Disconnected | Connected |
// FormatChanged`) and is named `AudioBackendBusEvent` to disambiguate.
//
// Wires four event variants:
//   - PriorNarrowed { rangeHz: [number, number] } -> setPriorRange
//   - Disconnected                                -> setDeviceStatus("disconnected")
//   - Connected                                   -> clearDeviceError +
//                                                    optional negotiated format
//   - FormatChanged { rateHz, channels }          -> setNegotiatedFormat +
//                                                    deviceStatus = "format_changed"
//
// Test surface: when running under the E2E mock the helper exposes
// `window.__neuralPitchTestHooks.listeners` keyed by `"audio:backend"`,
// which a spec drives via `pushDeviceEvent(page, event)`.
//
//   tests/e2e/helpers/tauri-mock.ts (synthetic event delivery)

import { useEffect } from "react";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getTestHooks, registerTestListener } from "@/lib/test-hooks";
import { useTunerStore } from "@/stores/tunerStore";

/** Wire-format audio-backend bus event. Field names mirror the Rust
 *  `serde` output (snake_case for nested data, but `type` + camelCase
 *  for the variant tag we control on the TS test harness side). */
export type AudioBackendBusEvent =
  | { type: "PriorNarrowed"; rangeHz: readonly [number, number] }
  | { type: "Disconnected" }
  | { type: "Connected"; rateHz?: number; channels?: number }
  | { type: "FormatChanged"; rateHz: number; channels: number };

const CHANNEL = "audio:backend";

function applyEvent(event: AudioBackendBusEvent): void {
  const store = useTunerStore.getState();
  switch (event.type) {
    case "PriorNarrowed":
      store.setPriorRange(event.rangeHz);
      return;
    case "Disconnected":
      store.setDeviceStatus("disconnected");
      return;
    case "Connected":
      if (typeof event.rateHz === "number" && typeof event.channels === "number") {
        store.setNegotiatedFormat({ rateHz: event.rateHz, channels: event.channels });
      }
      store.clearDeviceError();
      return;
    case "FormatChanged":
      store.setNegotiatedFormat({ rateHz: event.rateHz, channels: event.channels });
      store.setDeviceStatus("format_changed");
      return;
  }
}

/**
 * Subscribe to the `audio:backend` Tauri event for the lifetime of the
 * mounting component. Idempotent: only one subscription is active at a
 * time, even under React StrictMode double-invoke.
 */
export function useDeviceEvents(): void {
  useEffect(() => {
    let unlisten: UnlistenFn | null = null;
    let cancelled = false;

    // Prefer the Tauri `listen` API for production. Under the E2E mock
    // the page-side helper does not implement `plugin:event|listen`, so we
    // ALSO register through `__neuralPitchTestHooks.listeners` so specs
    // can drive synthetic events without needing the real listen wiring.
    const unregister = registerTestListener(CHANNEL, (payload) => {
      applyEvent(payload as AudioBackendBusEvent);
    });
    if (getTestHooks() === undefined) {
      void listen<AudioBackendBusEvent>(CHANNEL, (event) => {
        applyEvent(event.payload);
      })
        .then((u) => {
          if (cancelled) {
            u();
            return;
          }
          unlisten = u;
        })
        .catch(() => {
          /* swallow: production code should not crash on a missing
             event channel; the UI degrades to no auto-prior badge update. */
        });
    }

    return () => {
      cancelled = true;
      unregister();
      if (unlisten !== null) unlisten();
    };
  }, []);
}
