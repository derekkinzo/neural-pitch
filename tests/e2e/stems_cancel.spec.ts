// Phase 5 — StemSeparationPanel cancel-flow spec (TDD-RED).
//
// Drives the idle → separating → idle (cancelled) arc. Two synthetic
// progress frames fire, then the user clicks the
// `data-testid="cancel-separation"` Cancel button. The mock's
// `cancel_separation` handler rejects the parked `separate_stems`
// promise with an `Error("Cancelled")`, the panel returns to the idle
// state (Separate stems button visible again), and a polite
// `role="status"` toast carries "cancelled" copy.
//
// Mirrors the receiver-closed-early-tolerant pattern enforced for
// `pushMatchUpdate` and `pushStemsProgress`: the listener list is
// allowed to drain before the test fires its final transition, so the
// promise resolution drives the UI even if the channel listener has
// already torn down.
//

import { expect, test } from "./fixtures";
import {
  installRecordingsMock,
  installStemsMock,
  pushStemsProgress,
  type MockRecording,
} from "./helpers/tauri-mock";

const NOW = 1_700_000_000_000;

const REC_ID = "rec-stems-cancel-001";

const SEED: MockRecording[] = [
  {
    id: REC_ID,
    filename: "stems-cancel-001.flac",
    createdAt: NOW - 7 * 60 * 1000,
    durationMs: 7_000,
    sampleRateHz: 48000,
    channels: 1,
    bitDepth: 24,
    a4Hz: 440,
    instrumentProfile: "Voice",
  },
];

test.describe("phase 5 — stems cancel", () => {
  test("Cancel during separation returns the panel to idle", async ({ page, mockTauri }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installStemsMock(),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    await page.getByTestId("separate-stems").click();

    // Two mid-flight ticks exercise the channel path before cancel.
    await pushStemsProgress(page, { recordingId: REC_ID, stage: "vocals", percent: 10 });
    await pushStemsProgress(page, { recordingId: REC_ID, stage: "drums", percent: 35 });

    const cancel = page.getByTestId("cancel-separation");
    await expect(cancel).toBeVisible();
    await expect(cancel).toHaveAttribute("aria-keyshortcuts", /Escape/i);
    await cancel.click();

    // Panel returns to idle: Separate stems button is back.
    const separate = page.getByTestId("separate-stems");
    await expect(separate).toBeVisible();
    await expect(separate).toHaveText(/Separate stems/i);

    // The polite status region (role=status) carries the cancelled copy.
    // Multiple `role=status` regions co-exist (the global note-aria-live
    // is also a polite live region) so we target the panel-scoped one by
    // its stable test id rather than the role.
    const status = page.getByTestId("stems-status");
    await expect(status).toContainText(/cancelled/i);
  });
});
