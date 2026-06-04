// PermissionNotice — inline banner shown when the audio backend reports
// permission_denied. Rendered as a sibling of <main> in `Tuner.tsx` (only
// when `deviceStatus === "permission_denied"`) so it remains reachable to
// AT regardless of focus state.
//
// Body text branches on `navigator.userAgent` so macOS users see the
// platform-specific guidance.
//
// Cross-references:
//   docs/design/DESIGN.md §9.3 (audio backend events)
//   tests/e2e/permission.spec.ts

import { useEffect, useRef, useState, type ReactNode } from "react";

export interface PermissionNoticeProps {
  /** Re-issue start_capture with the cached settings. The hook returned
   *  by usePitchStream provides this. */
  onRetry: () => Promise<void>;
}

const MAC_GUIDANCE =
  "Open System Settings › Privacy & Security › Microphone and enable NeuralPitch.";
const GENERIC_GUIDANCE = "NeuralPitch needs microphone access to detect pitch.";

function isMacUserAgent(ua: string): boolean {
  // Catch both stable Mac UA strings and the iPad-on-Mac ambiguous form.
  return /Mac OS X|Macintosh/.test(ua);
}

export function PermissionNotice({ onRetry }: PermissionNoticeProps): ReactNode {
  const [busy, setBusy] = useState<boolean>(false);
  const retryButtonRef = useRef<HTMLButtonElement | null>(null);

  // Move focus to the Retry button once on mount so screen-magnifier users
  // (who track the system caret) immediately see the recovery action. We
  // do NOT trap focus — the banner is non-modal; users may Tab away. The
  // banner only mounts when deviceStatus transitions to permission_denied,
  // so this effect runs exactly once per denial event.
  useEffect(() => {
    retryButtonRef.current?.focus();
  }, []);

  const ua = typeof navigator !== "undefined" ? navigator.userAgent : "";
  const guidance = isMacUserAgent(ua) ? MAC_GUIDANCE : GENERIC_GUIDANCE;

  const handleRetry = async (): Promise<void> => {
    setBusy(true);
    try {
      await onRetry();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      role="alert"
      aria-live="assertive"
      data-testid="permission-notice"
      className="mx-6 mt-2 flex flex-col gap-2 rounded-md border border-amber-500/40 bg-amber-500/10 px-4 py-3 text-sm text-amber-100 sm:flex-row sm:items-center sm:justify-between"
    >
      <p data-testid="permission-guidance" className="leading-snug">
        {guidance}
      </p>
      {/* aria-busy on a button is non-canonical (it's intended for regions
          whose contents are being modified, not interactive controls);
          the disabled attribute already conveys "not interactable" and the
          visible "Retrying…" label conveys progress. NVDA / JAWS announce
          aria-busy on buttons inconsistently or redundantly. */}
      <button
        ref={retryButtonRef}
        type="button"
        aria-label="Retry microphone access"
        disabled={busy}
        onClick={() => {
          void handleRetry();
        }}
        className="self-start rounded-md border border-amber-400/60 bg-amber-500/20 px-3 py-1 text-xs font-medium text-amber-50 transition hover:bg-amber-500/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-amber-300 disabled:cursor-not-allowed disabled:opacity-50 sm:self-auto"
      >
        {busy ? "Retrying…" : "Retry"}
      </button>
    </div>
  );
}
