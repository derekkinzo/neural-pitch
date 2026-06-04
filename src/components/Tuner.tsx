// Tuner — top-level container for the Phase 1.2 / 1.3 / 2.0 tuner view.
//
// Owns:
//   - usePitchStream() — Channel<PitchUpdate> + ring buffer + retry()
//   - useDeviceEvents() — `audio:backend` event subscription
//   - useRecordingProgress() — `recording-progress` event subscription
//   - layout (header / NoteDisplay / CentsMeter / HistoryStrip)
//   - SettingsDrawer + RecordingsList drawer open/close state
//   - PermissionNotice + DeviceDisconnectToast + SavedToast (rendered as
//     siblings of <main>)
//
// Cross-references:
//   docs/design/DESIGN.md §1 (layout)
//   docs/design/DESIGN.md §7 (component tree)
//   docs/design/DESIGN.md §7.5 (Phase 2.0 frontend additions)
//   docs/design/DESIGN.md §9.3 (audio backend events)

import { useState, type ReactNode } from "react";
import { Button } from "@/components/ui/Button";
import { CentsMeter } from "@/components/CentsMeter";
import { DeviceDisconnectToast } from "@/components/DeviceDisconnectToast";
import { HistoryStrip } from "@/components/HistoryStrip";
import { NoteDisplay } from "@/components/NoteDisplay";
import { PermissionNotice } from "@/components/PermissionNotice";
import { RecordButton } from "@/components/recordings/RecordButton";
import { RecordingsList } from "@/components/recordings/RecordingsList";
import { SavedToast } from "@/components/recordings/SavedToast";
import { SettingsDrawer } from "@/components/SettingsDrawer";
import { StatusPill } from "@/components/StatusPill";
import { useDeviceEvents } from "@/hooks/useDeviceEvents";
import { usePitchStream } from "@/hooks/usePitchStream";
import { useRecordingProgress } from "@/hooks/useRecordingProgress";
import { useTunerStore } from "@/stores/tunerStore";

const GEAR_GLYPH = "⚙";
const LIBRARY_GLYPH = "♫";

export function Tuner(): ReactNode {
  const { ringRef, retry } = usePitchStream();
  useDeviceEvents();
  useRecordingProgress();

  const [settingsOpen, setSettingsOpen] = useState<boolean>(false);
  const [libraryOpen, setLibraryOpen] = useState<boolean>(false);
  const deviceStatus = useTunerStore((s) => s.deviceStatus);

  // The Settings + Recordings drawers are rendered as SIBLINGS of <main>
  // (not children) so each drawer's focus-trap can apply `inert` /
  // `aria-hidden="true"` to the entire main content without inadvertently
  // hiding the dialog itself. WAI-ARIA APG modal-dialog pattern requires
  // content outside the dialog to be inert.
  return (
    <>
      <main className="flex min-h-screen flex-col bg-slate-950 text-slate-50">
        {/* Header sits above the non-modal recordings drawer (z-40) so the
            record + library + settings triggers stay clickable while the
            drawer is open. The modal SettingsDrawer (z-50) still sits above
            the header — that path correctly inerts main and traps focus. */}
        <header className="relative z-50 flex items-center justify-between px-6 py-4">
          <StatusPill />
          <div className="flex items-center gap-2">
            <RecordButton />
            <Button
              variant="ghost"
              aria-label="Open recordings library"
              data-testid="library-trigger"
              onClick={() => setLibraryOpen(true)}
            >
              <span aria-hidden="true" className="text-lg">
                {LIBRARY_GLYPH}
              </span>
            </Button>
            <Button
              variant="ghost"
              aria-label="Open settings"
              data-testid="settings-trigger"
              onClick={() => setSettingsOpen(true)}
            >
              <span aria-hidden="true" className="text-lg">
                {GEAR_GLYPH}
              </span>
            </Button>
          </div>
        </header>

        {deviceStatus === "permission_denied" ? <PermissionNotice onRetry={retry} /> : null}

        <section className="flex flex-1 flex-col items-center justify-center gap-8 px-6">
          <NoteDisplay ringRef={ringRef} />
          <div className="w-full max-w-2xl">
            <CentsMeter ringRef={ringRef} />
          </div>
          <div className="w-full max-w-2xl">
            <HistoryStrip ringRef={ringRef} />
          </div>
        </section>
      </main>

      <SettingsDrawer open={settingsOpen} onOpenChange={setSettingsOpen} />
      <RecordingsList open={libraryOpen} onOpenChange={setLibraryOpen} />
      <DeviceDisconnectToast />
      <SavedToast />
    </>
  );
}
