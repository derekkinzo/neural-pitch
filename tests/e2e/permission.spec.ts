// PermissionNotice — banner shown when start_capture rejects with a
// permission_denied sentinel.
//
// The mock IPC handler is overridden to throw `"permission_denied"` until
// the spec flips a `permission_granted` flag on
// `window.__neuralPitchTestHooks`. The spec asserts:
//   1. The banner is visible with role="alert".
//   2. On macOS UA, the body text contains the System Settings guidance.
//   3. Clicking Retry re-invokes start_capture and the banner clears.
//
// React StrictMode mounts the tree twice in dev, so a count-based throw
// would let the second mount succeed and hide the banner. The flag-based
// approach is robust against StrictMode and against the cleanup-then-mount
// invocation order.
//
// Cross-references:
//   docs/design/TEST-PLAN.md §6.2 (a11y category)
//   docs/design/DESIGN.md §9.3 (permission denied recovery)

import { test, expect } from "./fixtures";
import { getInvokeCalls } from "./helpers/tauri-mock";

test.describe("permission — denied banner + retry", () => {
  test("shows macOS guidance and retries on click", async ({ page, mockTauri, axe }) => {
    // Spoof the macOS UA before any script runs so the PermissionNotice
    // body picks the platform-specific guidance.
    await page.addInitScript(() => {
      Object.defineProperty(navigator, "userAgent", {
        configurable: true,
        get: () =>
          "Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15",
      });
    });

    // The mock returns a permission_denied error until a flag is flipped
    // (see the retry click handler below). Flag lives on the page-side
    // hooks object so the function form survives toString-serialisation.
    await mockTauri.install({
      start_capture: () => {
        const w = window as unknown as {
          __neuralPitchTestHooks?: { permission_granted?: boolean };
        };
        const granted = w.__neuralPitchTestHooks?.permission_granted === true;
        if (!granted) {
          throw new Error("permission_denied");
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

    const banner = page.getByRole("alert");
    await expect(banner).toBeVisible();
    await expect(banner).toContainText(/System Settings/);
    await expect(banner).toContainText(/Privacy & Security/);

    const beforeRetry = await getInvokeCalls(page, "start_capture");
    expect(beforeRetry.length).toBeGreaterThanOrEqual(1);

    // Now grant permission so the next start_capture resolves.
    await page.evaluate(() => {
      const w = window as unknown as {
        __neuralPitchTestHooks?: { permission_granted?: boolean };
      };
      if (w.__neuralPitchTestHooks !== undefined) {
        w.__neuralPitchTestHooks.permission_granted = true;
      }
    });

    // Retry button — accessible name set by `aria-label`.
    const retry = page.getByRole("button", { name: "Retry microphone access" });
    await expect(retry).toBeVisible();
    const beforeClickCount = (await getInvokeCalls(page, "start_capture")).length;
    await retry.click();

    // The retry path issues stop_capture followed by start_capture, so
    // the start_capture invocation count must increase by at least one.
    await expect
      .poll(async () => (await getInvokeCalls(page, "start_capture")).length, { timeout: 2000 })
      .toBeGreaterThan(beforeClickCount);

    // The banner is gone once permission is granted.
    await expect(banner).toHaveCount(0);

    const results = await axe.analyze();
    const blocking = results.violations.filter(
      (v) => v.impact === "serious" || v.impact === "critical",
    );
    expect(blocking, `axe violations:\n${JSON.stringify(blocking, null, 2)}`).toEqual([]);
  });
});
