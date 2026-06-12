// DeviceDisconnectToast — bottom-right toast shown when the audio backend
// reports a Disconnected event. Auto-dismisses on Connected (which clears
// `deviceStatus` back to "ok" through useDeviceEvents).
//
// The Reconnect button invokes `configure({ device: "default" })` which on
// the Rust side triggers a stop_capture / start_capture cycle.
//

import { useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useTunerStore } from "@/stores/tunerStore";

export function DeviceDisconnectToast(): ReactNode {
  const deviceStatus = useTunerStore((s) => s.deviceStatus);
  const [busy, setBusy] = useState<boolean>(false);

  if (deviceStatus !== "disconnected") return null;

  const reconnect = async (): Promise<void> => {
    setBusy(true);
    try {
      await invoke("configure", { device: "default" });
      // Optimistically clear the status; the Connected backend event will
      // also clear it via useDeviceEvents.
      useTunerStore.getState().clearDeviceError();
    } catch {
      /* swallow: a follow-up Disconnected event will keep the toast up. */
    } finally {
      setBusy(false);
    }
  };

  // role="alert" implies aria-live="assertive" + aria-atomic="true": a
  // device disconnect carries a destructive recovery action and SHOULD
  // be announced immediately rather than queued behind speech the way a
  // polite live region would (see WAI-ARIA APG live-region pattern).
  return (
    <div
      role="alert"
      data-testid="disconnect-toast"
      className="fixed bottom-4 right-4 flex items-center gap-3 rounded-md border border-rose-500/40 bg-slate-900/95 px-4 py-3 text-sm text-slate-100 shadow-lg"
    >
      <span>Audio device disconnected.</span>
      <button
        type="button"
        aria-label="Reconnect to default microphone"
        disabled={busy}
        onClick={() => {
          void reconnect();
        }}
        className="rounded-md border border-rose-400/60 bg-rose-500/20 px-3 py-1 text-xs font-medium text-rose-50 transition hover:bg-rose-500/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rose-300 disabled:cursor-not-allowed disabled:opacity-50"
      >
        {busy ? "Reconnecting…" : "Reconnect"}
      </button>
    </div>
  );
}
