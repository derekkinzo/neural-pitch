// Phase 2.1 — RecordingDetail accessibility scan.
//
// Mirrors `recordings_a11y.spec.ts` but focused on the RecordingDetail
// regions (header, AnalysisSummary card, ContourLine `<figure>`). The
// scan runs after a row is selected so the detail panel is mounted
// inside the drawer body. We assert zero serious / critical axe-core
// violations against WCAG 2.1 AA.
//

import { expect, test } from "./fixtures";
import {
  installAnalysisMock,
  installRecordingsMock,
  type MockAnalysisSummary,
  type MockContourResult,
  type MockRecording,
} from "./helpers/tauri-mock";

const NOW = 1_700_000_000_000;

const SEED: MockRecording[] = [
  {
    id: "rec-a11y-detail-001",
    filename: "axe-detail-001.flac",
    createdAt: NOW - 6 * 60 * 1000,
    durationMs: 14_500,
    sampleRateHz: 48000,
    channels: 1,
    bitDepth: 24,
    a4Hz: 440,
    instrumentProfile: "Voice",
  },
];

const SUMMARY: Record<string, MockAnalysisSummary> = {
  "rec-a11y-detail-001": {
    recordingId: "rec-a11y-detail-001",
    medianMidi: 69,
    medianCents: 1.2,
    voicedRatio: 0.91,
    wasCached: true,
    analyzerVersion: "pyin-0.1.0",
  },
};

const CONTOUR: Record<string, MockContourResult> = {
  "rec-a11y-detail-001:pyin-0.1.0": {
    recordingId: "rec-a11y-detail-001",
    analyzerVersion: "pyin-0.1.0",
    medianMidi: 69,
    medianCents: 1.2,
    voicedRatio: 0.91,
    frames: [
      { tMs: 0, centsFromMedian: -3, voiced: true },
      { tMs: 100, centsFromMedian: 0, voiced: true },
      { tMs: 200, centsFromMedian: 2, voiced: true },
      { tMs: 300, centsFromMedian: 4, voiced: true },
    ],
  },
};

test.describe("a11y — recording detail panel", () => {
  test("axe scan reports no serious or critical violations", async ({ page, mockTauri, axe }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMock(SUMMARY, CONTOUR),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("library-trigger").click();
    const list = page.getByTestId("recordings-list");
    await expect(list).toBeVisible();
    await page.getByTestId("recording-row").first().click();

    // Wait for the detail panel to mount before scanning.
    await expect(page.getByTestId("recording-detail-header")).toBeVisible();
    await expect(page.getByRole("group", { name: /Analysis summary/i })).toBeVisible();

    const results = await axe.analyze();
    const blocking = results.violations.filter(
      (v) => v.impact === "serious" || v.impact === "critical",
    );
    expect(blocking, `axe violations:\n${JSON.stringify(blocking, null, 2)}`).toEqual([]);
  });

  test("ContourLine exposes role=img with composed aria-label", async ({ page, mockTauri }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMock(SUMMARY, CONTOUR),
    });
    await page.goto("/");
    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    // The semantic role is on the wrapper, not the <canvas>; per
    // The canvas itself is aria-hidden.
    const figure = page.getByRole("img", { name: /Pitch contour/i });
    await expect(figure).toBeVisible();

    const canvas = page.locator("canvas[data-testid=contour-canvas]");
    await expect(canvas).toHaveAttribute("aria-hidden", "true");
  });

  test("Re-analyze button exposes a button role with the expected label", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMock(SUMMARY, CONTOUR),
    });
    await page.goto("/");
    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    const button = page.getByTestId("reanalyze");
    await expect(button).toBeVisible();
    const tag = await button.evaluate((el) => el.tagName.toLowerCase());
    const role = await button.getAttribute("role");
    expect(tag === "button" || role === "button").toBe(true);
    await expect(button).toHaveText(/Re-analyze/i);
  });
});
