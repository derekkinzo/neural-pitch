// Tuner — top-level container for the Phase 1.2 / 1.3 live-tuner view.
//
// Owns:
//   - usePitchStream() — Channel<PitchUpdate> + ring buffer + retry()
//   - useDeviceEvents() — `audio:backend` event subscription
//   - layout (header / NoteDisplay / CentsMeter / HistoryStrip)
//   - SettingsDrawer open/close state
//   - PermissionNotice + DeviceDisconnectToast (rendered as siblings of <main>)
//
// Cross-references:
//   docs/design/DESIGN.md §1 (layout)
//   docs/design/DESIGN.md §7 (component tree)
//   docs/design/DESIGN.md §9.3 (audio backend events)

import { useState, type ReactNode } from "react";
import { Button } from "@/components/ui/Button";
import { CentsMeter } from "@/components/CentsMeter";
import { DeviceDisconnectToast } from "@/components/DeviceDisconnectToast";
import { HistoryStrip } from "@/components/HistoryStrip";
import { NoteDisplay } from "@/components/NoteDisplay";
import { PermissionNotice } from "@/components/PermissionNotice";
import { SettingsDrawer } from "@/components/SettingsDrawer";
import { StatusPill } from "@/components/StatusPill";
import { useDeviceEvents } from "@/hooks/useDeviceEvents";
import { usePitchStream } from "@/hooks/usePitchStream";
import { useTunerStore } from "@/stores/tunerStore";

const GEAR_GLYPH = "⚙";

export function Tuner(): ReactNode {
  const { ringRef, retry } = usePitchStream();
  useDeviceEvents();
  const [settingsOpen, setSettingsOpen] = useState<boolean>(false);
  const deviceStatus = useTunerStore((s) => s.deviceStatus);

  // The SettingsDrawer is rendered as a SIBLING of <main> (not a child)
  // so the drawer's focus-trap can apply `inert` / `aria-hidden="true"` to
  // the entire main content without inadvertently hiding the dialog
  // itself. WAI-ARIA APG modal-dialog pattern requires content outside
  // the dialog to be inert; co-locating the drawer inside <main> would
  // make the drawer unreachable to AT.
  return (
    <>
      <main className="flex min-h-screen flex-col bg-slate-950 text-slate-50">
        <header className="flex items-center justify-between px-6 py-4">
          <StatusPill />
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
      <DeviceDisconnectToast />
    </>
  );
}
