// Phase 2.3 — RangeReadout spec.
//
// Asserts the seeded recording row, when clicked, mounts the new
// RangeReadout component as a sibling of AnalysisSummary inside
// RecordingDetail (in a 2-column grid alongside VibratoReadout, *below*
// the summary card and above ContourLine). Three branches:
//
//   1. Happy path — comfortable + full ranges render as note pairs
//      formatted via the recording's own `a4Hz`, voice-type hint pills
//      land, and the New Grove tooltip text is reachable.
//   2. Empty / insufficient state — `voicedFrameCount` below the
//      ~250-frame threshold renders the single "Not enough voiced
//      material..." paragraph and skips the pills.
//   3. Update budget — exactly one `analyze_recording` invoke fires per
//      recording open; the cached summary carries `range` directly so
//      no second IPC round-trip happens.
//
// All payloads come from the Phase-2.3 mocks; no real IPC fires.
//
//   tests/e2e/recording_detail.spec.ts (sibling spec; same row → click flow)

import { expect, test } from "./fixtures";
import {
  getInvokeCalls,
  installAnalysisMockWithRange,
  installRecordingsMock,
  type MockAnalysisSummary,
  type MockContourResult,
  type MockRangeReport,
  type MockRecording,
} from "./helpers/tauri-mock";

const NOW = 1_700_000_000_000;

const SEED: MockRecording[] = [
  {
    id: "rec-range-001",
    filename: "range-take-001.flac",
    createdAt: NOW - 5 * 60 * 1000,
    durationMs: 18_000,
    sampleRateHz: 48000,
    channels: 1,
    bitDepth: 24,
    a4Hz: 440,
    instrumentProfile: "Voice",
  },
];

const SUMMARY: Record<string, MockAnalysisSummary> = {
  "rec-range-001": {
    recordingId: "rec-range-001",
    medianMidi: 64,
    medianCents: 0.0,
    voicedRatio: 0.91,
    wasCached: true,
    analyzerVersion: "pyin-0.1.0",
  },
};

const CONTOUR: Record<string, MockContourResult> = {
  "rec-range-001:pyin-0.1.0": {
    recordingId: "rec-range-001",
    analyzerVersion: "pyin-0.1.0",
    medianMidi: 64,
    medianCents: 0.0,
    voicedRatio: 0.91,
    frames: [
      { tMs: 0, centsFromMedian: -2, voiced: true },
      { tMs: 100, centsFromMedian: 0, voiced: true },
      { tMs: 200, centsFromMedian: 3, voiced: true },
      { tMs: 300, centsFromMedian: 1, voiced: true },
    ],
  },
};

// Comfortable C4–F5 with Alto / Mezzo-soprano hints (per New Grove).
const RANGE_HAPPY: Record<string, MockRangeReport> = {
  "rec-range-001": {
    comfortableLowMidi: 60, // C4
    comfortableHighMidi: 65, // F5? No — MIDI 65 is F4; the spec's example
    // is C4 → F5, which is MIDI 60 → 77. We follow the spec literal.
    fullLowMidi: 57, // A3
    fullHighMidi: 81, // A5
    voicedFrameCount: 540,
    voiceTypeHints: ["Alto", "Mezzo-soprano"],
  },
};

// Override the comfortable high to match the spec's explicit "C4 - F5" pair
// (MIDI 60 → 77). The constant above flagged the off-by-octave; correct here.
RANGE_HAPPY["rec-range-001"]!.comfortableHighMidi = 77;

// Below the ~250-frame voiced-material threshold.
const RANGE_INSUFFICIENT: Record<string, MockRangeReport> = {
  "rec-range-001": {
    comfortableLowMidi: 60,
    comfortableHighMidi: 62,
    fullLowMidi: 60,
    fullHighMidi: 63,
    voicedFrameCount: 42,
    voiceTypeHints: [],
  },
};

test.describe("range readout — comfortable + full + voice hints", () => {
  test("happy path renders C4 - F5 comfortable, A3 - A5 full, and hint pills", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMockWithRange(SUMMARY, CONTOUR, RANGE_HAPPY),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("library-trigger").click();
    await expect(page.getByTestId("recordings-list")).toBeVisible();
    await page.getByTestId("recording-row").first().click();

    const region = page.getByRole("group", { name: /Vocal range report/i });
    await expect(region).toBeVisible();
    await expect(page.getByTestId("range-readout")).toBeVisible();

    // Comfortable range — C4 - F5 (MIDI 60 → 77 at A4=440 Hz).
    await expect(page.getByTestId("range-comfortable")).toHaveText(/C4\s*-\s*F5/);
    // Full range — A3 - A5 (MIDI 57 → 81).
    await expect(page.getByTestId("range-full")).toHaveText(/A3\s*-\s*A5/);
    // Voiced-frame count surfaced as informational text.
    await expect(page.getByTestId("range-voiced-frames")).toContainText(/540/);

    // Two voice-type hint pills (Alto, Mezzo-soprano).
    const pills = page.getByTestId("voice-hint-pill");
    await expect(pills).toHaveCount(2);
    await expect(pills.nth(0)).toContainText(/Alto/i);
    await expect(pills.nth(1)).toContainText(/Mezzo-soprano/i);

    // The New Grove disclaimer is reachable somewhere in the region —
    // either via the tooltip content or via the visually-hidden fallback
    // paragraph for AT users that swallow tooltip content.
    await expect(region).toContainText(/NOT a vocal coach assessment/i);
  });

  test("empty / insufficient state renders single 'Not enough voiced material' paragraph", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMockWithRange(SUMMARY, CONTOUR, RANGE_INSUFFICIENT),
    });
    await page.goto("/");
    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    const region = page.getByRole("group", { name: /Vocal range report/i });
    await expect(region).toBeVisible();

    const empty = page.getByTestId("range-empty");
    await expect(empty).toBeVisible();
    await expect(empty).toContainText(/Not enough voiced material/i);
    await expect(empty).toContainText(/at least 5 seconds/i);

    // Pills must be skipped in the empty branch.
    await expect(page.getByTestId("voice-hint-pill")).toHaveCount(0);
    // The numeric readouts are also suppressed.
    await expect(page.getByTestId("range-comfortable")).toHaveCount(0);
    await expect(page.getByTestId("range-full")).toHaveCount(0);
  });

  test("update budget — opening the row fires exactly one analyze_recording", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMockWithRange(SUMMARY, CONTOUR, RANGE_HAPPY),
    });
    await page.goto("/");
    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    // Wait for the readout to mount before sampling the call log.
    await expect(page.getByTestId("range-readout")).toBeVisible();

    const calls = await getInvokeCalls(page, "analyze_recording");
    expect(calls).toHaveLength(1);
  });
});
