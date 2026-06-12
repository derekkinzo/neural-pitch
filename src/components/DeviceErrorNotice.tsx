// DeviceErrorNotice — inline banner shown when the audio backend fails to
// initialise for a non-permission reason (no microphone connected, ALSA
// card missing, cpal init error, etc.). Sibling of <main> in `Tuner.tsx`,
// rendered when `tunerStore.startError !== null` so AT users land on the
// recovery action regardless of focus state.
//
// Distinct from `PermissionNotice`: that one fires on the explicit
// `permission_denied` sentinel, while this one catches the general
// "something else went wrong starting the audio device" path. The
// StatusPill paints `data-state="error"` for the same condition; the
// banner is the prominent counterpart so users do not have to notice the
// small red dot in the corner.
//
//   tests/e2e/device_error_notice.spec.ts

import { useEffect, useRef, useState, type ReactNode } from "react";

const HEADING = "Microphone unavailable";
const BODY =
  "Could not initialise the audio backend. Check that a microphone is connected and that the OS lets this app use it.";

export interface DeviceErrorNoticeProps {
  /** Re-issue start_capture with the cached settings. The hook returned
   *  by usePitchStream provides this; it also clears the previous
   *  startError so the banner unmounts on a successful retry. */
  onRetry: () => Promise<void>;
}

export function DeviceErrorNotice({ onRetry }: DeviceErrorNoticeProps): ReactNode {
  const [busy, setBusy] = useState<boolean>(false);
  const retryButtonRef = useRef<HTMLButtonElement | null>(null);

  // Move focus to Retry on mount so screen-magnifier users (who track the
  // system caret) immediately see the recovery action. Non-modal — we do
  // not trap focus; users may Tab away. The banner only mounts when
  // startError transitions to a non-null value, so this runs once per
  // failure event.
  useEffect(() => {
    retryButtonRef.current?.focus();
  }, []);

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
      data-testid="device-error-notice"
      className="mx-6 mt-2 flex flex-col gap-2 rounded-md border border-rose-500/40 bg-rose-500/10 px-4 py-3 text-sm text-rose-100 sm:flex-row sm:items-start sm:justify-between"
    >
      <div className="flex flex-col gap-1">
        <p
          data-testid="device-error-heading"
          className="text-sm font-semibold leading-snug text-rose-50"
        >
          {HEADING}
        </p>
        <p data-testid="device-error-body" className="leading-snug">
          {BODY}
        </p>
      </div>
      {/* aria-busy on a button is non-canonical (it targets regions whose
          contents are being modified, not interactive controls); the
          disabled attribute already conveys "not interactable" and the
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
        className="self-start rounded-md border border-rose-400/60 bg-rose-500/20 px-3 py-1 text-xs font-medium text-rose-50 transition hover:bg-rose-500/30 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-rose-300 disabled:cursor-not-allowed disabled:opacity-50 sm:self-auto"
      >
        {busy ? "Retrying…" : "Retry"}
      </button>
    </div>
  );
}
