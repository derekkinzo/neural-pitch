// SavedToast — bottom-right transient confirmation rendered after
// `stop_recording` resolves.
//
// Auto-dismisses 4 s after surfacing. Reuses the same fixed bottom-right
// slot as `<DeviceDisconnectToast>`; only one of the two is visible at a
// time because a disconnect already nukes capture state and the user would
// not be in mid-recording when the disconnect fires. The two surfaces sit
// side-by-side without conflicting because the disconnect-toast is a
// `role="alert"` (assertive) and this one is a `role="status"` (polite).
//
// Cross-references:
//   docs/design/DESIGN.md §7.5 (Phase 2.0 frontend additions — saved toast)

import { useEffect, type ReactNode } from "react";
import { useRecordingsStore } from "@/stores/recordingsStore";

const TOAST_TIMEOUT_MS = 4_000;

export function SavedToast(): ReactNode {
  const message = useRecordingsStore((s) => s.savedToastMessage);
  const dismiss = useRecordingsStore((s) => s.dismissSavedToast);

  useEffect(() => {
    if (message === null) return;
    const id = window.setTimeout(() => dismiss(), TOAST_TIMEOUT_MS);
    return () => {
      window.clearTimeout(id);
    };
  }, [message, dismiss]);

  if (message === null) return null;

  return (
    <div
      role="status"
      // Explicit `aria-live="polite"` mirrors the implicit value of
      // `role="status"` but is verbose enough that linters / future
      // refactors can't drop the polite-announcement guarantee. The
      // inner `<span>` is keyed by the message text so back-to-back
      // saves within the 4 s timeout remount the announce-target — AT
      // re-reads the message instead of seeing a no-op text update on
      // a still-mounted node.
      aria-live="polite"
      aria-atomic="true"
      data-testid="saved-toast"
      className="fixed bottom-4 right-4 flex items-center gap-3 rounded-md border border-emerald-500/40 bg-slate-900/95 px-4 py-3 text-sm text-slate-100 shadow-lg"
    >
      <span key={message}>{message}</span>
    </div>
  );
}
