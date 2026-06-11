// StemSeparationPanel accessibility spec.
//
// Mirrors `playback_a11y.spec.ts` and `training_a11y.spec.ts` but scans
// three discrete states of the StemSeparationPanel:
//
//   1. idle           — Separate stems button alone
//   2. mid-separating — progress bar + Cancel button
//   3. complete       — four StemCards with PlaybackPanels
//
// Contract: zero `serious` / `critical` violations against WCAG 2.1 AA.
// Each pass scopes the axe analyse to the panel subtree so failures
// point at the component under test.
//

import { expect, test } from "./fixtures";
import {
  installRecordingsMock,
  installStemsMock,
  pushStemsComplete,
  pushStemsProgress,
  type MockRecording,
} from "./helpers/tauri-mock";

const NOW = 1_700_000_000_000;

const REC_ID = "rec-stems-a11y-001";

const SEED: MockRecording[] = [
  {
    id: REC_ID,
    filename: "stems-a11y-001.flac",
    createdAt: NOW - 9 * 60 * 1000,
    durationMs: 4_000,
    sampleRateHz: 48000,
    channels: 1,
    bitDepth: 24,
    a4Hz: 440,
    instrumentProfile: "Voice",
  },
];

async function selectSeededRecording(page: import("@playwright/test").Page): Promise<void> {
  await page.goto("/");
  await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");
  await page.getByTestId("library-trigger").click();
  await page.getByTestId("recording-row").first().click();
}

test.describe("a11y — stems panel", () => {
  test("idle stems panel reports no serious or critical violations", async ({
    page,
    mockTauri,
    axe,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installStemsMock(),
    });
    await selectSeededRecording(page);

    await expect(page.getByTestId("stem-separation-panel")).toBeVisible();
    await expect(page.getByTestId("separate-stems")).toBeVisible();

    const results = await axe.include('[data-testid="stem-separation-panel"]').analyze();
    const blocking = results.violations.filter(
      (v) => v.impact === "serious" || v.impact === "critical",
    );
    expect(blocking, `axe violations:\n${JSON.stringify(blocking, null, 2)}`).toEqual([]);
  });

  test("mid-separating stems panel reports no serious or critical violations", async ({
    page,
    mockTauri,
    axe,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installStemsMock(),
    });
    await selectSeededRecording(page);

    await page.getByTestId("separate-stems").click();
    await pushStemsProgress(page, { recordingId: REC_ID, stage: "drums", percent: 47 });

    await expect(
      page.getByRole("progressbar", { name: /Stem separation progress/i }),
    ).toBeVisible();
    await expect(page.getByTestId("cancel-separation")).toBeVisible();

    const results = await axe.include('[data-testid="stem-separation-panel"]').analyze();
    const blocking = results.violations.filter(
      (v) => v.impact === "serious" || v.impact === "critical",
    );
    expect(blocking, `axe violations:\n${JSON.stringify(blocking, null, 2)}`).toEqual([]);
  });

  test("complete stems panel reports no serious or critical violations", async ({
    page,
    mockTauri,
    axe,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installStemsMock(),
    });
    await selectSeededRecording(page);

    await page.getByTestId("separate-stems").click();
    await pushStemsProgress(page, { recordingId: REC_ID, stage: "drums", percent: 47 });
    await pushStemsComplete(page, { recordingId: REC_ID });

    await expect(page.locator('[data-testid^="stem-card-"]')).toHaveCount(4);

    const results = await axe.include('[data-testid="stem-separation-panel"]').analyze();
    const blocking = results.violations.filter(
      (v) => v.impact === "serious" || v.impact === "critical",
    );
    expect(blocking, `axe violations:\n${JSON.stringify(blocking, null, 2)}`).toEqual([]);
  });
});
