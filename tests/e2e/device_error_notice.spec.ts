// DeviceErrorNotice — banner shown when start_capture rejects with a
// non-permission error (no microphone connected, ALSA card missing, cpal
// init failure, etc.). Distinct from permission.spec.ts which exercises
// the dedicated `permission_denied` sentinel branch.
//
// The mock IPC handler is overridden to throw `"no audio device"` until
// the spec flips a `device_recovered` flag on
// `window.__neuralPitchTestHooks`. The spec asserts:
//   1. StatusPill paints `data-state="error"` when start_capture fails.
//   2. The DeviceErrorNotice banner is visible with the expected heading.
//   3. Clicking Retry re-invokes start_capture and the banner clears.
//
// React StrictMode mounts the tree twice in dev, so a count-based throw
// would let the second mount succeed and hide the banner. The flag-based
// approach is robust against StrictMode and against the cleanup-then-mount
// invocation order. Mirrors permission.spec.ts.
//

import { test, expect } from "./fixtures";
import { getInvokeCalls } from "./helpers/tauri-mock";

test.describe("device-error — banner + retry", () => {
  test("shows the error banner and StatusPill error state, then retries", async ({
    page,
    mockTauri,
  }) => {
    // The mock returns a generic audio-backend error until a flag is
    // flipped (see the retry click handler below). Flag lives on the
    // page-side hooks object so the function form survives toString-
    // serialisation.
    await mockTauri.install({
      start_capture: () => {
        const w = window as unknown as {
          __neuralPitchTestHooks?: { device_recovered?: boolean };
        };
        const recovered = w.__neuralPitchTestHooks?.device_recovered === true;
        if (!recovered) {
          throw new Error("no audio device");
        }
        return {
          device_name: "Mock Microphone",
          sample_rate_hz: 48000,
          window_samples: 2048,
          hop_samples: 512,
          channels: 1,
        };
      },
    });

    await page.goto("/");

    // StatusPill flips to data-state="error" because the failed start
    // routes the error string into `tunerStore.startError`.
    const pill = page.getByTestId("status-pill");
    await expect(pill).toHaveAttribute("data-state", "error");

    // The banner is the prominent counterpart of the small red dot.
    const banner = page.getByTestId("device-error-notice");
    await expect(banner).toBeVisible();
    await expect(banner).toHaveAttribute("role", "alert");
    await expect(page.getByTestId("device-error-heading")).toHaveText("Microphone unavailable");
    await expect(banner).toContainText(/Could not initialise the audio backend/);

    const beforeClickCount = (await getInvokeCalls(page, "start_capture")).length;
    expect(beforeClickCount).toBeGreaterThanOrEqual(1);

    // Now flip the flag so the next start_capture resolves cleanly.
    await page.evaluate(() => {
      const w = window as unknown as {
        __neuralPitchTestHooks?: { device_recovered?: boolean };
      };
      if (w.__neuralPitchTestHooks !== undefined) {
        w.__neuralPitchTestHooks.device_recovered = true;
      }
    });

    // Retry button — accessible name set by `aria-label`. Mirrors the
    // PermissionNotice contract (same label text, distinct banner).
    const retry = page.getByRole("button", { name: "Retry microphone access" });
    await expect(retry).toBeVisible();
    await retry.click();

    // The retry path issues stop_capture followed by start_capture, so
    // the start_capture invocation count must increase by at least one.
    await expect
      .poll(async () => (await getInvokeCalls(page, "start_capture")).length, { timeout: 2000 })
      .toBeGreaterThan(beforeClickCount);

    // Once the retry resolves, the store's `setCaptureStarted` clears
    // `startError` and the banner unmounts; the StatusPill flips back
    // out of the error state.
    await expect(banner).toHaveCount(0);
    await expect(pill).not.toHaveAttribute("data-state", "error");
  });
});
