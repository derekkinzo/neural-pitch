// StemSeparationPanel progress spec.
//
// Drives the idle → separating transition. Five synthetic
// `separate-progress` frames map to the documented stage cycle
// (vocals → drums → bass → other → finalizing). After each frame the
// `<progress role="progressbar">` aria-valuenow / aria-valuetext
// recompute deterministically so a screen-reader (and this spec)
// observe a stable readout.
//
// The hot-path percent is published at ~10–20 Hz; production ducks
// React re-renders by writing through a ref + rAF. The spec only
// checks the post-frame steady state, so it is robust to that
// optimisation and to receiver-closed-early.
//

import { expect, test } from "./fixtures";
import {
  installRecordingsMock,
  installStemsMock,
  pushStemsProgress,
  type MockRecording,
  type MockSeparateProgress,
} from "./helpers/tauri-mock";

const NOW = 1_700_000_000_000;

const REC_ID = "rec-stems-progress-001";

const SEED: MockRecording[] = [
  {
    id: REC_ID,
    filename: "stems-progress-001.flac",
    createdAt: NOW - 5 * 60 * 1000,
    durationMs: 5_000,
    sampleRateHz: 48000,
    channels: 1,
    bitDepth: 24,
    a4Hz: 440,
    instrumentProfile: "Voice",
  },
];

const FRAMES: MockSeparateProgress[] = [
  { recordingId: REC_ID, stage: "vocals", percent: 10 },
  { recordingId: REC_ID, stage: "drums", percent: 30 },
  { recordingId: REC_ID, stage: "bass", percent: 50 },
  { recordingId: REC_ID, stage: "other", percent: 70 },
  { recordingId: REC_ID, stage: "finalizing", percent: 90 },
];

const STAGE_LABEL: Record<MockSeparateProgress["stage"], RegExp> = {
  vocals: /Separating vocals/i,
  drums: /Separating drums/i,
  bass: /Separating bass/i,
  other: /Separating other/i,
  finalizing: /Finalizing/i,
};

test.describe("stems progress", () => {
  test("progress bar tracks aria-valuenow and aria-valuetext per stage", async ({
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

    const progress = page.getByRole("progressbar", {
      name: /Stem separation progress/i,
    });
    await expect(progress).toBeVisible();

    // Drive the five staged frames; assert the steady-state aria
    // attributes recompute after each.
    for (const frame of FRAMES) {
      await pushStemsProgress(page, frame);
      await expect(progress).toHaveAttribute("aria-valuenow", String(frame.percent));
      await expect(progress).toHaveAttribute(
        "aria-valuetext",
        new RegExp(`${frame.percent}\\s*percent`, "i"),
      );
      await expect(progress).toHaveAttribute("aria-valuetext", STAGE_LABEL[frame.stage]);
    }

    // The polite live region carries human-readable status copy that
    // mirrors the latest stage transition. Only stage transitions write
    // here — the per-frame percent does NOT, per the panel contract.
    const status = page.getByTestId("stems-status");
    await expect(status).toContainText(/Finalizing|Separating other/i);
  });
});
