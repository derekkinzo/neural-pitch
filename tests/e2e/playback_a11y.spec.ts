// PlaybackPanel axe scan.
//
// Mirrors `recording_a11y.spec.ts` but focused on the new wavesurfer
// surface. After the panel mounts we run axe-core scoped via the
// fixture's `axe` builder (WCAG 2.1 AA tags) and assert zero serious /
// critical violations on the panel subtree.

import { expect, test } from "./fixtures";
import {
  installPlaybackMock,
  installPlaybackRoutes,
  installRecordingsMock,
  type MockRecording,
} from "./helpers/tauri-mock";

const NOW = 1_700_000_000_000;

const SEED: MockRecording[] = [
  {
    id: "rec-playback-a11y-001",
    filename: "axe-playback-001.flac",
    createdAt: NOW - 2 * 60 * 1000,
    durationMs: 1_000,
    sampleRateHz: 8000,
    channels: 1,
    bitDepth: 16,
    a4Hz: 440,
    instrumentProfile: "Voice",
  },
];

test.describe("a11y — playback panel", () => {
  test("axe scan reports no serious or critical violations", async ({ page, mockTauri, axe }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installPlaybackMock(),
    });
    await installPlaybackRoutes(page);

    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("library-trigger").click();
    await expect(page.getByTestId("recordings-list")).toBeVisible();
    await page.getByTestId("recording-row").first().click();

    const panel = page.getByTestId("playback-panel");
    await expect(panel).toBeVisible();
    // Wait for wavesurfer canvas to mount before scanning so the analyser
    // sees the final DOM, including the play/pause button and slider.
    await expect(panel.locator("canvas").first()).toBeVisible({ timeout: 2000 });
    await expect(page.getByTestId("playback-toggle")).toBeVisible();
    await expect(page.getByTestId("spectrogram-toggle")).toBeVisible();

    const results = await axe.include('[data-testid="playback-panel"]').analyze();
    const blocking = results.violations.filter(
      (v) => v.impact === "serious" || v.impact === "critical",
    );
    expect(blocking, `axe violations:\n${JSON.stringify(blocking, null, 2)}`).toEqual([]);
  });
});
