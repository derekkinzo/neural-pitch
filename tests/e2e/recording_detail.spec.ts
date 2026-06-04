// Phase 2.1 — RecordingDetail spec.
//
// Asserts the seeded recording row, when clicked, mounts the new
// RecordingDetail component below RecordingsList in the same drawer body
// (sibling of PlaybackPanel). Three regions render:
//
//   1. <header data-testid=recording-detail-header>: filename, formatted
//      duration, relative createdAt, instrument-profile badge, A4 pill.
//   2. AnalysisSummary (`role=group` aria-label="Analysis summary"):
//      median note, median cents (signed, 1 decimal), voiced ratio (%),
//      and a was_cached badge ("Cached" or "Fresh").
//   3. ContourLine (`<figure>` with `<canvas data-testid=contour-canvas>`)
//      with a wrapping `role=img` aria-label composed from the summary.
//
// All payloads come from the Phase-2.1 mocks; no real IPC fires.
//
// Cross-references:
//   docs/design/DESIGN.md §7.5 (Phase-2 frontend additions)
//   docs/design/DESIGN.md §8.3 (analysis_cache schema)
//   docs/adr/0006-visual-only-feedback-prefers-reduced-motion.md
//   src/components/CentsMeter.tsx (canonical canvas + DPR + reduced-motion pattern)

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
    id: "rec-detail-001",
    filename: "voice-detail-001.flac",
    createdAt: NOW - 8 * 60 * 1000,
    durationMs: 12_000,
    sampleRateHz: 48000,
    channels: 1,
    bitDepth: 24,
    a4Hz: 442,
    instrumentProfile: "Voice",
  },
];

const SUMMARY: Record<string, MockAnalysisSummary> = {
  "rec-detail-001": {
    recordingId: "rec-detail-001",
    medianMidi: 69, // A4
    medianCents: -3.5,
    voicedRatio: 0.92,
    wasCached: true,
    analyzerVersion: "pyin-0.1.0",
  },
};

// Deterministic 12-frame contour. Mix of voiced + a single unvoiced gap so
// the polyline restart path is exercised by the eventual production canvas
// drawer (the spec only asserts the canvas mounts; the gap/path details
// are covered by component tests later).
const CONTOUR: Record<string, MockContourResult> = {
  "rec-detail-001:pyin-0.1.0": {
    recordingId: "rec-detail-001",
    analyzerVersion: "pyin-0.1.0",
    medianMidi: 69,
    medianCents: -3.5,
    voicedRatio: 0.92,
    frames: [
      { tMs: 0, centsFromMedian: -10, voiced: true },
      { tMs: 100, centsFromMedian: -5, voiced: true },
      { tMs: 200, centsFromMedian: 0, voiced: true },
      { tMs: 300, centsFromMedian: 4, voiced: true },
      { tMs: 400, centsFromMedian: 8, voiced: true },
      { tMs: 500, centsFromMedian: 0, voiced: false }, // unvoiced gap
      { tMs: 600, centsFromMedian: -2, voiced: true },
      { tMs: 700, centsFromMedian: -6, voiced: true },
      { tMs: 800, centsFromMedian: -8, voiced: true },
      { tMs: 900, centsFromMedian: -3, voiced: true },
      { tMs: 1000, centsFromMedian: 1, voiced: true },
      { tMs: 1100, centsFromMedian: 5, voiced: true },
    ],
  },
};

test.describe("recording detail — header + summary + contour", () => {
  test("clicking a row mounts header with filename, duration, and A4 pill", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMock(SUMMARY, CONTOUR),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("library-trigger").click();
    const list = page.getByTestId("recordings-list");
    await expect(list).toBeVisible();

    const row = page.getByTestId("recording-row").first();
    await row.click();

    const header = page.getByTestId("recording-detail-header");
    await expect(header).toBeVisible();
    await expect(header).toContainText("voice-detail-001.flac");
    // 12000 ms → "0:12" via formatDurationShort.
    await expect(header).toContainText(/0:12|12\s*s/);
    await expect(header).toContainText(/Voice/);
    // A4 pill mirrors the recording's tuning reference (442 Hz here).
    await expect(header).toContainText(/A4\s*=\s*442\s*Hz/);
  });

  test("AnalysisSummary card surfaces median note, cents, voiced ratio, cache badge", async ({
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
    await expect(summary).toBeVisible();

    // Median note: MIDI 69 → "A4".
    await expect(summary).toContainText(/A4/);
    // Median cents: signed, 1 decimal — "-3.5".
    await expect(summary).toContainText(/-3\.5/);
    // Voiced ratio: 0.92 → "92%".
    await expect(summary).toContainText(/92\s*%/);
    // wasCached=true seed → "Cached" badge.
    await expect(summary).toContainText(/Cached/);
  });

  test("ContourLine renders canvas with non-zero client size", async ({ page, mockTauri }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMock(SUMMARY, CONTOUR),
    });
    await page.goto("/");
    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    const figure = page.locator("figure", { has: page.locator("[data-testid=contour-canvas]") });
    await expect(figure).toBeVisible();

    const canvas = page.locator("canvas[data-testid=contour-canvas]");
    await expect(canvas).toBeVisible();
    // The canvas is decorative; semantic state lives on the wrapping role=img.
    await expect(canvas).toHaveAttribute("aria-hidden", "true");

    const dims = await canvas.evaluate((el) => ({
      w: (el as HTMLCanvasElement).clientWidth,
      h: (el as HTMLCanvasElement).clientHeight,
    }));
    expect(dims.w).toBeGreaterThan(0);
    expect(dims.h).toBeGreaterThan(0);

    // The wrapping role=img element carries the composed aria-label.
    const figureRole = page.getByRole("img", { name: /Pitch contour/i });
    await expect(figureRole).toBeVisible();
  });
});
