// ImportButton — Phase 3 file-import affordance.
//
// Lives in the `RecordingsList` drawer-header toolbar beside `RecordButton`.
// Flat React component with no local state — the open/close lifecycle of
// the Tauri dialog is fire-and-forget and the resulting recording row
// arrives through `recordingsStore.refresh()` rather than a useState.
//
// In-flight UX: the button reads `aria-busy="true"` and is `disabled`
// while either the dialog or the IPC is pending so a double-click cannot
// fire `import_audio_file` twice.
//
// Failure paths route through `recordingsStore.lastError`, exactly like
// a failed `stop_recording` — we don't surface a separate toast for
// import errors, mirroring the existing pattern.

import { useState, type ReactNode } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openAudioFileDialog } from "@/lib/dialog";
import { useRecordingsStore } from "@/stores/recordingsStore";
import type { Recording } from "@/types/recording";

const FILTERS = [{ name: "Audio", extensions: ["wav", "flac", "mp3"] }] as const;

export function ImportButton(): ReactNode {
  const refresh = useRecordingsStore((s) => s.refresh);
  const setError = useRecordingsStore((s) => s.setError);

  const [busy, setBusy] = useState<boolean>(false);

  const onClick = async (): Promise<void> => {
    if (busy) return;
    setBusy(true);
    try {
      const sourcePath = await openAudioFileDialog(FILTERS);
      if (typeof sourcePath !== "string") {
        // User dismissed the dialog — silent no-op, no IPC fired.
        return;
      }
      // The Rust shell returns the persisted Recording row; we still
      // refresh the list so the Zustand-backed listbox picks up the new
      // entry with sort applied.
      await invoke<Recording>("import_audio_file", { sourcePath });
      await refresh();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : String(err);
      setError(msg);
    } finally {
      setBusy(false);
    }
  };

  // Reuse the focus / hover styling from RecordButton so the toolbar
  // reads as a single visual unit.
  const baseBtn =
    "relative inline-flex h-8 items-center justify-center rounded-md border px-3 " +
    "text-xs font-medium transition-colors focus-visible:outline-none " +
    "focus-visible:ring-2 focus-visible:ring-cyan-300 focus-visible:ring-offset-2 " +
    "focus-visible:ring-offset-slate-950 disabled:cursor-not-allowed disabled:opacity-60";
  const stateBtn = busy
    ? "border-slate-600 bg-slate-800 text-slate-200"
    : "border-cyan-500/40 bg-slate-900/60 text-cyan-200 hover:bg-cyan-500/10";

  return (
    <button
      type="button"
      data-testid="import-button"
      aria-label="Import audio file"
      aria-busy={busy}
      disabled={busy}
      onClick={() => {
        void onClick();
      }}
      className={[baseBtn, stateBtn].join(" ")}
    >
      <span aria-hidden="true" className="mr-1">
        ↥
      </span>
      Import
    </button>
  );
}
