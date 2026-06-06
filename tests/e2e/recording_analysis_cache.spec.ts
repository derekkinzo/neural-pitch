// Phase 2.1 — Analysis cache + Re-analyze spec.
//
// Drives the wasCached → forceRefresh → wasCached=false transition through
// the AnalysisSummary card. Two assertions:
//
//   1. Initial click on a seeded row resolves with `was_cached=true` and
//      the card shows the "Cached" badge.
//   2. Pressing [data-testid=reanalyze] swaps the numeric readouts for an
//      AnalysisProgress `<progress role="progressbar">`. After
//      `pushAnalysisProgress(percent=100)` resolves the in-flight
//      analyze_recording call, the badge re-renders as "Fresh".
//

import { expect, test } from "./fixtures";
import {
  installAnalysisMock,
  installRecordingsMock,
  pushAnalysisProgress,
  type MockAnalysisSummary,
  type MockContourResult,
  type MockRecording,
} from "./helpers/tauri-mock";

const NOW = 1_700_000_000_000;

const SEED: MockRecording[] = [
  {
    id: "rec-cache-001",
    filename: "cache-take-001.flac",
    createdAt: NOW - 4 * 60 * 1000,
    durationMs: 9_000,
    sampleRateHz: 48000,
    channels: 1,
    bitDepth: 24,
    a4Hz: 440,
    instrumentProfile: "Voice",
  },
];

// `wasCached: true` here is the seed default; the mock handler overrides it
// to `!forceRefresh` per call so both branches share a single seed.
const SUMMARY: Record<string, MockAnalysisSummary> = {
  "rec-cache-001": {
    recordingId: "rec-cache-001",
    medianMidi: 69,
    medianCents: 0.0,
    voicedRatio: 0.88,
    wasCached: true,
    analyzerVersion: "pyin-0.1.0",
  },
};

const CONTOUR: Record<string, MockContourResult> = {
  "rec-cache-001:pyin-0.1.0": {
    recordingId: "rec-cache-001",
    analyzerVersion: "pyin-0.1.0",
    medianMidi: 69,
    medianCents: 0.0,
    voicedRatio: 0.88,
    frames: [
      { tMs: 0, centsFromMedian: -2, voiced: true },
      { tMs: 100, centsFromMedian: 0, voiced: true },
      { tMs: 200, centsFromMedian: 3, voiced: true },
      { tMs: 300, centsFromMedian: 1, voiced: true },
    ],
  },
};

test.describe("recording analysis cache — cached badge + re-analyze", () => {
  test("first open shows Cached badge from was_cached=true", async ({ page, mockTauri }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMock(SUMMARY, CONTOUR),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    const summary = page.getByRole("group", { name: /Analysis summary/i });
    await expect(summary).toBeVisible();
    await expect(summary).toContainText(/Cached/);
    // The "Fresh" branch is mutually exclusive — make sure both badges aren't
    // rendered at once. (The card re-renders on each analyze call.)
    await expect(summary).not.toContainText(/Fresh/);
  });

  test("Re-analyze swaps summary for progress bar, then renders Fresh", async ({
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

    const summary = page.getByRole("group", { name: /Analysis summary/i });
    await expect(summary).toContainText(/Cached/);

    // Press Re-analyze. The card replaces the numeric readouts with a
    // <progress role="progressbar"> labelled "Analyzing recording".
    const reanalyze = page.getByTestId("reanalyze");
    await expect(reanalyze).toBeVisible();
    await reanalyze.click();

    const progress = page.getByRole("progressbar", { name: /Analyzing recording/i });
    await expect(progress).toBeVisible();

    // Drive a few progress ticks; the bar can update its `value` attribute.
    await pushAnalysisProgress(page, { recordingId: "rec-cache-001", percent: 25 });
    await pushAnalysisProgress(page, { recordingId: "rec-cache-001", percent: 75 });

    // Resolve the analysis. The mock returned wasCached=false because
    // forceRefresh=true was passed; once the in-flight promise resolves
    // the card re-renders the numeric readouts and the badge flips.
    await pushAnalysisProgress(page, { recordingId: "rec-cache-001", percent: 100 });

    await expect(progress).toBeHidden();
    await expect(summary).toContainText(/Fresh/);
    await expect(summary).not.toContainText(/Cached/);
  });
});
