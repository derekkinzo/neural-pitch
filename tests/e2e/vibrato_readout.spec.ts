// Phase 2.3 — VibratoReadout spec.
//
// Asserts the seeded recording row, when clicked, mounts the new
// VibratoReadout component to the right of RangeReadout in the same
// 2-column grid (sibling of AnalysisSummary; below summary, above
// ContourLine). Four branches:
//
//   1. Happy path — median rate 5.4 Hz lands the meter at
//      `aria-valuenow="5.4"` with `aria-valuetext` matching
//      /typical voice range/i, and the 4–7 Hz typical band rect renders.
//   2. Per-window dot strip — one dot per `windows[i]`.
//   3. Reduced-motion — when `prefers-reduced-motion: reduce` matches,
//      the indicator carries a `transition-none` class (no animated
//      movement).
//   4. Empty state — `vibratoRatio < 0.05` renders the single
//      "No vibrato detected." paragraph.
//
// All payloads come from the Phase-2.3 mocks; no real IPC fires.
//
//   src/components/CentsMeter.tsx (canonical reduced-motion pattern)

import { expect, test } from "./fixtures";
import {
  installAnalysisMockWithVibrato,
  installRecordingsMock,
  type MockAnalysisSummary,
  type MockContourResult,
  type MockRecording,
  type MockVibratoReport,
} from "./helpers/tauri-mock";

const NOW = 1_700_000_000_000;

const SEED: MockRecording[] = [
  {
    id: "rec-vibrato-001",
    filename: "vibrato-take-001.flac",
    createdAt: NOW - 3 * 60 * 1000,
    durationMs: 16_000,
    sampleRateHz: 48000,
    channels: 1,
    bitDepth: 24,
    a4Hz: 440,
    instrumentProfile: "Voice",
  },
];

const SUMMARY: Record<string, MockAnalysisSummary> = {
  "rec-vibrato-001": {
    recordingId: "rec-vibrato-001",
    medianMidi: 67,
    medianCents: 0.0,
    voicedRatio: 0.93,
    wasCached: true,
    analyzerVersion: "pyin-0.1.0",
  },
};

const CONTOUR: Record<string, MockContourResult> = {
  "rec-vibrato-001:pyin-0.1.0": {
    recordingId: "rec-vibrato-001",
    analyzerVersion: "pyin-0.1.0",
    medianMidi: 67,
    medianCents: 0.0,
    voicedRatio: 0.93,
    frames: [
      { tMs: 0, centsFromMedian: -2, voiced: true },
      { tMs: 100, centsFromMedian: 1, voiced: true },
      { tMs: 200, centsFromMedian: 4, voiced: true },
      { tMs: 300, centsFromMedian: 0, voiced: true },
    ],
  },
};

// Median 5.4 Hz lies inside the 4–7 Hz typical band; ratio 0.32 clears
// the empty-state threshold (>= 0.05). Three windows seed the dot strip.
const VIBRATO_HAPPY: Record<string, MockVibratoReport> = {
  "rec-vibrato-001": {
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

// vibratoRatio below the 0.05 empty-state threshold.
const VIBRATO_EMPTY: Record<string, MockVibratoReport> = {
  "rec-vibrato-001": {
    medianRateHz: 0.0,
    medianExtentCents: 0,
    vibratoRatio: 0.02,
    windows: [],
  },
};

test.describe("vibrato readout — meter + dots + reduced-motion + empty", () => {
  test("happy path renders meter at 5.4 Hz inside typical band, with numeric readouts", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMockWithVibrato(SUMMARY, CONTOUR, VIBRATO_HAPPY),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    const region = page.getByRole("group", { name: /Vibrato analysis/i });
    await expect(region).toBeVisible();
    await expect(page.getByTestId("vibrato-readout")).toBeVisible();

    // Numeric readouts: rate (1 decimal Hz), extent (cents), ratio (%).
    await expect(page.getByTestId("vibrato-rate")).toContainText(/5\.4/);
    await expect(page.getByTestId("vibrato-extent")).toContainText(/32/);
    await expect(page.getByTestId("vibrato-ratio")).toContainText(/32\s*%/);

    // role="meter" with valuemin/valuemax + valuenow + valuetext.
    const meter = region.getByRole("meter");
    await expect(meter).toHaveAttribute("aria-valuemin", "0");
    await expect(meter).toHaveAttribute("aria-valuemax", "10");
    await expect(meter).toHaveAttribute("aria-valuenow", "5.4");
    await expect(meter).toHaveAttribute("aria-valuetext", /typical voice range/i);

    // The typical-voice band rect is visible under the indicator.
    await expect(page.getByTestId("typical-band")).toBeVisible();
  });

  test("per-window strip renders one dot per windows[i]", async ({ page, mockTauri }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMockWithVibrato(SUMMARY, CONTOUR, VIBRATO_HAPPY),
    });
    await page.goto("/");
    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    await expect(page.getByTestId("vibrato-readout")).toBeVisible();

    // 3 seeded windows → 3 dots.
    const dots = page.getByTestId("vibrato-window-dot");
    await expect(dots).toHaveCount(3);
  });

  test("prefers-reduced-motion: reduce → indicator has transition-none class", async ({
    page,
    mockTauri,
  }) => {
    // Emulate before navigation so the very first render sees the
    // reduced-motion media query as matching (per CentsMeter pattern).
    await page.emulateMedia({ reducedMotion: "reduce" });

    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMockWithVibrato(SUMMARY, CONTOUR, VIBRATO_HAPPY),
    });
    await page.goto("/");
    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    const region = page.getByRole("group", { name: /Vibrato analysis/i });
    await expect(region).toBeVisible();

    // The indicator element (the static <line> the rate bar paints over
    // the typical-band rect) carries the transition-none class when
    // reduced motion is active. Locate it by data-testid rather than a
    // class-substring match so the assertion does not silently match the
    // wrapping role="meter" div (which carries the same class). See the
    // VibratoReadout file header for the full reduced-motion contract.
    const indicator = region.getByTestId("vibrato-indicator");
    await expect(indicator).toHaveClass(/transition-none/);
  });

  test("default motion → indicator carries transition-all class", async ({ page, mockTauri }) => {
    // Symmetric assertion against the reduced-motion branch above — pin
    // both motion classes so a future refactor that drops one branch
    // breaks loudly rather than silently.
    await page.emulateMedia({ reducedMotion: "no-preference" });

    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMockWithVibrato(SUMMARY, CONTOUR, VIBRATO_HAPPY),
    });
    await page.goto("/");
    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    const region = page.getByRole("group", { name: /Vibrato analysis/i });
    await expect(region).toBeVisible();

    const indicator = region.getByTestId("vibrato-indicator");
    await expect(indicator).toHaveClass(/transition-all/);
  });

  test("empty state — vibratoRatio < 0.05 renders 'No vibrato detected.'", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMockWithVibrato(SUMMARY, CONTOUR, VIBRATO_EMPTY),
    });
    await page.goto("/");
    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    const region = page.getByRole("group", { name: /Vibrato analysis/i });
    await expect(region).toBeVisible();

    const empty = page.getByTestId("vibrato-empty");
    await expect(empty).toBeVisible();
    await expect(empty).toContainText(/No vibrato detected/i);

    // Numeric readouts + meter + dots all suppressed in the empty branch.
    await expect(page.getByTestId("vibrato-rate")).toHaveCount(0);
    await expect(page.getByTestId("vibrato-window-dot")).toHaveCount(0);
    await expect(region.getByRole("meter")).toHaveCount(0);
  });
});
