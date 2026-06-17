// StemSeparationPanel error-branch + retry spec.
//
// Drives the idle → separating → error → (retry) → complete arc. The
// parked `separate_stems` promise is rejected with a non-cancellation
// error via `pushStemsError`; the store flips to `status: "error"` (a
// "Cancelled" message would route to idle instead), so the panel renders
// the `role="alert"` failure paragraph and the `stems-retry` button.
//
// Clicking Retry re-issues `separate()`, which parks a fresh promise; a
// subsequent `pushStemsComplete` resolves it and the panel progresses
// past idle into the complete branch — proving the recovery path wires
// back into a working separation.
//

import { expect, test } from "./fixtures";
import {
  installRecordingsMock,
  installStemsMock,
  pushStemsComplete,
  pushStemsError,
  pushStemsProgress,
  type MockRecording,
} from "./helpers/tauri-mock";

const NOW = 1_700_000_000_000;

const REC_ID = "rec-stems-error-001";

const SEED: MockRecording[] = [
  {
    id: REC_ID,
    filename: "stems-error-001.flac",
    createdAt: NOW - 8 * 60 * 1000,
    durationMs: 8_000,
    sampleRateHz: 48000,
    channels: 1,
    bitDepth: 24,
    a4Hz: 440,
    instrumentProfile: "Voice",
  },
];

test.describe("stems error", () => {
  test("separation failure shows the alert + Retry recovers to complete", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installStemsMock(),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    await page.getByTestId("separate-stems").click();

    // One mid-flight tick, then the parked promise rejects with a
    // non-cancellation error — the store lands in the `error` branch.
    await pushStemsProgress(page, { recordingId: REC_ID, stage: "vocals", percent: 15 });
    await pushStemsError(page, { recordingId: REC_ID, message: "separation failed" });

    // The error paragraph is a polite alert carrying the failure copy.
    const panel = page.getByTestId("stem-separation-panel");
    const failureAlert = panel.getByRole("alert").filter({ hasText: /Stem separation failed/i });
    await expect(failureAlert).toBeVisible();
    await expect(failureAlert).toContainText(/separation failed/i);

    // The Retry button is the only control on the error branch.
    const retry = page.getByTestId("stems-retry");
    await expect(retry).toBeVisible();
    await expect(retry).toHaveText(/Retry/i);

    await retry.click();

    // Retry re-issues separate(); the panel leaves the error branch and a
    // fresh complete resolves into the StemCards.
    await pushStemsComplete(page, { recordingId: REC_ID });

    // The complete branch is live: StemCards mount and the error-branch
    // chrome (the failure alert + Retry button) is gone. (The nested
    // per-stem PlaybackPanels emit their own load-failure alerts here
    // because no audio routes are wired, so we target the panel-level
    // failure copy specifically rather than all alerts.)
    await expect(page.getByTestId("stem-card-vocals")).toBeVisible();
    await expect(page.getByTestId("stems-retry")).toHaveCount(0);
    await expect(
      panel.getByRole("alert").filter({ hasText: /Stem separation failed/i }),
    ).toHaveCount(0);
  });
});
