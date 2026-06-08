// Phase 2.3 — RangeReadout + VibratoReadout accessibility scan.
//
// Mirrors `recording_a11y.spec.ts` but focused on the two new readouts
// mounted between AnalysisSummary and ContourLine. We assert zero
// serious / critical axe-core violations against WCAG 2.1 AA across
// both `role="group"` regions when both reports are seeded.
//

import { expect, test } from "./fixtures";
import {
  installAnalysisMockWithRange,
  installAnalysisMockWithVibrato,
  installRecordingsMock,
  type MockAnalysisSummary,
  type MockContourResult,
  type MockRangeReport,
  type MockRecording,
  type MockVibratoReport,
} from "./helpers/tauri-mock";

const NOW = 1_700_000_000_000;

const SEED: MockRecording[] = [
  {
    id: "rec-rv-a11y-001",
    filename: "axe-rv-001.flac",
    createdAt: NOW - 7 * 60 * 1000,
    durationMs: 17_000,
    sampleRateHz: 48000,
    channels: 1,
    bitDepth: 24,
    a4Hz: 440,
    instrumentProfile: "Voice",
  },
];

const SUMMARY: Record<string, MockAnalysisSummary> = {
  "rec-rv-a11y-001": {
    recordingId: "rec-rv-a11y-001",
    medianMidi: 67,
    medianCents: 0.7,
    voicedRatio: 0.94,
    wasCached: true,
    analyzerVersion: "pyin-0.1.0",
  },
};

const CONTOUR: Record<string, MockContourResult> = {
  "rec-rv-a11y-001:pyin-0.1.0": {
    recordingId: "rec-rv-a11y-001",
    analyzerVersion: "pyin-0.1.0",
    medianMidi: 67,
    medianCents: 0.7,
    voicedRatio: 0.94,
    frames: [
      { tMs: 0, centsFromMedian: -2, voiced: true },
      { tMs: 100, centsFromMedian: 0, voiced: true },
      { tMs: 200, centsFromMedian: 3, voiced: true },
      { tMs: 300, centsFromMedian: 1, voiced: true },
    ],
  },
};

const RANGE: Record<string, MockRangeReport> = {
  "rec-rv-a11y-001": {
    comfortableLowMidi: 60, // C4
    comfortableHighMidi: 77, // F5
    fullLowMidi: 57, // A3
    fullHighMidi: 81, // A5
    voicedFrameCount: 612,
    voiceTypeHints: ["Alto", "Mezzo-soprano"],
  },
};

const VIBRATO: Record<string, MockVibratoReport> = {
  "rec-rv-a11y-001": {
    medianRateHz: 5.4,
    medianExtentCents: 32,
    vibratoRatio: 0.32,
    windows: [
      { tMs: 0, rateHz: 5.2, extentCents: 28, confidence: 0.4 },
      { tMs: 250, rateHz: 5.5, extentCents: 33, confidence: 0.7 },
      { tMs: 500, rateHz: 5.6, extentCents: 35, confidence: 0.92 },
    ],
  },
};

test.describe("a11y — range + vibrato readouts", () => {
  test("axe scan reports no serious or critical violations across both readouts", async ({
    page,
    mockTauri,
    axe,
  }) => {
    // Both wrappers funnel through `installAnalysisMock`; spreading the
    // vibrato wrapper after the range wrapper preserves the `range`
    // field because both wrappers re-derive the merged summary from the
    // same base `byRecordingId` map. Each wrapper installs its own
    // analyze_recording handler with its own embedded seed; we install
    // the combined range+vibrato variant directly via a single seed
    // map that carries both fields.
    const summaryWithBoth: Record<
      string,
      MockAnalysisSummary & { range: MockRangeReport; vibrato: MockVibratoReport }
    > = {
      "rec-rv-a11y-001": {
        ...SUMMARY["rec-rv-a11y-001"]!,
        range: RANGE["rec-rv-a11y-001"]!,
        vibrato: VIBRATO["rec-rv-a11y-001"]!,
      },
    };
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      // Single install: range wrapper consumes the `range` seed; the
      // resulting summary in the page already carries `vibrato` because
      // we promote it through the base summary map.
      ...installAnalysisMockWithRange(
        summaryWithBoth as Record<string, MockAnalysisSummary>,
        CONTOUR,
        RANGE,
      ),
    });
    // Sanity — the second wrapper is exported and importable; ensures
    // a future spec that toggles to vibrato-only doesn't suffer a dead
    // import. The result is intentionally unused here.
    void installAnalysisMockWithVibrato;

    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("library-trigger").click();
    await expect(page.getByTestId("recordings-list")).toBeVisible();
    await page.getByTestId("recording-row").first().click();

    // Both readout regions must mount before we scan. axe is a static
    // analyzer; if the regions are not yet present, the scan is a
    // no-op and would produce a false green.
    await expect(page.getByRole("group", { name: /Pitch range report/i })).toBeVisible();
    await expect(page.getByRole("group", { name: /Vibrato analysis/i })).toBeVisible();

    const results = await axe.analyze();
    const blocking = results.violations.filter(
      (v) => v.impact === "serious" || v.impact === "critical",
    );
    expect(blocking, `axe violations:\n${JSON.stringify(blocking, null, 2)}`).toEqual([]);
  });
});
