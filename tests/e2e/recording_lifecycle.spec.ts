// RecordButton lifecycle spec.
//
// Drives the idle → recording → saving lifecycle through the mock IPC
// surface and asserts the visible affordances:
//
//   1. Pressing [data-testid=record-button] in the idle state starts a
//      recording (state flips to "recording", aria-pressed=true).
//   2. Pressing it again stops the recording, the saved-toast surfaces with
//      text matching /Saved \d+s recording/, and `[data-testid=recordings-list]
//      li` count grows by one.
//   3. With `prefers-reduced-motion: reduce` emulation, the pulsing red dot
//      is suppressed (no CSS animation on the pulse element) and the
//      mm:ss elapsed counter is the visual cue.
//

import { expect, test } from "./fixtures";
import {
  installRecordingsMock,
  pushRecordingProgress,
  type MockRecording,
} from "./helpers/tauri-mock";

const SEED: MockRecording[] = [];

test.describe("recording lifecycle — start/stop/save", () => {
  test("idle → recording → saved toast → list grows by 1", async ({ page, mockTauri }) => {
    await mockTauri.install({ ...installRecordingsMock(SEED) });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    // Open the recordings drawer first so we can observe the initial row
    // count without racing the drawer-open animation later.
    await page.getByTestId("library-trigger").click();
    const list = page.getByTestId("recordings-list");
    await expect(list).toBeVisible();
    const initialRows = await list.locator("li").count();

    // Idle button is rendered in the tuner header.
    const button = page.getByTestId("record-button");
    await expect(button).toBeVisible();
    await expect(button).toHaveAttribute("data-state", "idle");
    await expect(button).toHaveAttribute("aria-pressed", "false");

    // Click → recording.
    await button.click();
    await expect(button).toHaveAttribute("data-state", "recording");
    await expect(button).toHaveAttribute("aria-pressed", "true");

    // Drive a single progress tick so the elapsed counter shows non-zero
    // before stop is pressed.
    await pushRecordingProgress(page, {
      recordingId: "rec-pending",
      elapsedMs: 1230,
      sampleCount: 48000,
      droppedWindows: 0,
      status: "active",
    });

    // 500 ms wait per the brief, gives the elapsed counter a frame to tick.
    await page.waitForTimeout(500);

    // Click → saving → saved.
    await button.click();

    // Toast surfaces in the bottom-right slot with the formatted duration.
    const toast = page.getByText(/Saved \d+s recording/);
    await expect(toast).toBeVisible();

    // List grows by exactly one row.
    await expect(list.locator("li")).toHaveCount(initialRows + 1);

    // Button cycles back to idle.
    await expect(button).toHaveAttribute("data-state", "idle");
    await expect(button).toHaveAttribute("aria-pressed", "false");
  });

  test("aria-label tracks elapsed time while recording", async ({ page, mockTauri }) => {
    await mockTauri.install({ ...installRecordingsMock(SEED) });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    const button = page.getByTestId("record-button");
    await expect(button).toHaveAttribute("aria-label", /Start recording/i);

    await button.click();
    await expect(button).toHaveAttribute("data-state", "recording");

    // Push a progress frame at 1m23s = 83000 ms. The label should regenerate
    // to read "Stop recording (1:23)" so AT users hear the elapsed time on
    // re-focus, without spamming the live-region speech queue.
    await pushRecordingProgress(page, {
      recordingId: "rec-pending",
      elapsedMs: 83000,
      sampleCount: 48000 * 83,
      droppedWindows: 0,
      status: "active",
    });

    await expect(button).toHaveAttribute("aria-label", /Stop recording \(1:23\)/);
  });

  test("prefers-reduced-motion suppresses the pulse animation", async ({ page, mockTauri }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await mockTauri.install({ ...installRecordingsMock(SEED) });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    const button = page.getByTestId("record-button");
    await button.click();
    await expect(button).toHaveAttribute("data-state", "recording");

    // The pulse element carries data-testid="record-pulse". In reduced-motion
    // mode its CSS animation-name resolves to "none" and the elapsed counter
    // is the only visual cue.
    const pulse = page.getByTestId("record-pulse");
    await expect(pulse).toBeVisible();
    const animationName = await pulse.evaluate((el) => getComputedStyle(el).animationName);
    expect(animationName).toBe("none");

    // The elapsed counter is rendered next to the button in reduced-motion
    // mode (and also in motion mode, per the brief — the cue swaps, not the
    // presence). Driving a progress frame should make it tick.
    await pushRecordingProgress(page, {
      recordingId: "rec-pending",
      elapsedMs: 5000,
      sampleCount: 48000 * 5,
      droppedWindows: 0,
      status: "active",
    });
    const elapsed = page.getByTestId("record-elapsed");
    await expect(elapsed).toContainText(/0:0[5-9]|0:05/);
  });
});
