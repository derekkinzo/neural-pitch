// Tuner — top-level container for the live tuner view.
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

import { useState, type ReactNode } from "react";
import { Button } from "@/components/ui/Button";
import { CentsMeter } from "@/components/CentsMeter";
import { DeviceDisconnectToast } from "@/components/DeviceDisconnectToast";
import { HistoryStrip } from "@/components/HistoryStrip";
import { NoteDisplay } from "@/components/NoteDisplay";
import { PermissionNotice } from "@/components/PermissionNotice";
import { ImportButton } from "@/components/recordings/ImportButton";
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
const PRACTICE_GLYPH = "♪";

export function Tuner(): ReactNode {
  const { ringRef, retry } = usePitchStream();
  useDeviceEvents();
  useRecordingProgress();

  const [settingsOpen, setSettingsOpen] = useState<boolean>(false);
  const [libraryOpen, setLibraryOpen] = useState<boolean>(false);
  const deviceStatus = useTunerStore((s) => s.deviceStatus);
  const setView = useTunerStore((s) => s.setView);

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
            the header — that path correctly inerts main and traps focus.
            When the non-modal drawer is open the header reserves the
            drawer's 360 px column on the right (`pr-[376px]` = 360 + 16 px
            gutter) so the toolbar buttons no longer fight the drawer panel
            for the same screen real estate. */}
        <header
          className={
            libraryOpen
              ? "relative z-50 flex items-center justify-between py-4 pl-6 pr-[376px] transition-[padding] duration-150"
              : "relative z-50 flex items-center justify-between px-6 py-4 transition-[padding] duration-150"
          }
        >
          <StatusPill />
          <div className="flex items-center gap-2">
            {/* Toolbar: co-locates the record + import controls
                under a single `role="toolbar"` group so AT users land on
                a labelled landmark before navigating into either control.
                The drawer and settings triggers stay outside the toolbar
                — they are navigation, not part of the recording surface. */}
            <div
              role="toolbar"
              aria-label="Recording controls"
              data-testid="recording-toolbar"
              className="flex items-center gap-2"
            >
              <RecordButton />
              <ImportButton />
            </div>
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
              aria-label="Open ear-training drills"
              data-testid="practice-trigger"
              onClick={() => setView("training")}
            >
              <span aria-hidden="true" className="text-lg">
                {PRACTICE_GLYPH}
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
