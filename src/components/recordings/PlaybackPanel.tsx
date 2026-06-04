// PlaybackPanel — Phase 2.0 stub for in-drawer recording playback.
//
// The Phase-2.4 surface ships WaveSurfer.js + a spectrogram overlay. Phase
// 2.0 deliberately limits the panel to a vanilla `<audio controls>` element
// fed via `convertFileSrc(absolutePath)` — that path is the documented
// interop bridge from a Tauri-resolved filesystem path to a `src` attribute
// the browser can stream.
//
// The panel is `hidden` (literally `display: none`) until a recording is
// selected. We render the markup unconditionally so the drawer DOM shape
// stays stable across selection changes (avoids layout-shift / focus-trap
// recompute).
//
// Cross-references:
//   docs/design/DESIGN.md §7.5 phase-additions table

import { useEffect, useState, type ReactNode } from "react";
import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import { useRecordingsStore } from "@/stores/recordingsStore";

export function PlaybackPanel(): ReactNode {
  const currentRecordingId = useRecordingsStore((s) => s.currentRecordingId);
  const [src, setSrc] = useState<string | null>(null);
  const [errorMsg, setErrorMsg] = useState<string | null>(null);

  useEffect(() => {
    if (currentRecordingId === null) {
      setSrc(null);
      setErrorMsg(null);
      return;
    }
    let cancelled = false;
    void (async (): Promise<void> => {
      try {
        const path = await invoke<string>("get_recording_path", { id: currentRecordingId });
        if (cancelled) return;
        setSrc(convertFileSrc(path));
        setErrorMsg(null);
      } catch (err: unknown) {
        if (cancelled) return;
        const msg = err instanceof Error ? err.message : String(err);
        setErrorMsg(msg);
        setSrc(null);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [currentRecordingId]);

  if (currentRecordingId === null) return null;

  return (
    <section
      data-testid="playback-panel"
      aria-label="Recording playback"
      className="flex flex-col gap-2 rounded-md border border-slate-700 bg-slate-900/50 p-3"
    >
      <div className="text-xs font-medium uppercase tracking-wide text-slate-300">Playback</div>
      {errorMsg !== null ? (
        <div role="alert" className="text-xs text-rose-300">
          Could not load recording: {errorMsg}
        </div>
      ) : null}
      {src !== null ? (
        // Captions are not authored at the recording layer; this is a raw
        // playback surface for user-recorded takes (Phase 2.4 will replace
        // the bare element with WaveSurfer.js + spectrogram).
        <audio data-testid="playback-audio" controls src={src} className="w-full">
          <track kind="captions" />
        </audio>
      ) : (
        <div className="text-xs text-slate-500">Loading…</div>
      )}
    </section>
  );
}
