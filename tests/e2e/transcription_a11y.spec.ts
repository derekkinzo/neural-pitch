// TranscribePanel + PianoRoll accessibility scan.
//
// Mirrors `recording_a11y.spec.ts` but focused on the transcription
// regions: TranscribePanel (button + cached badge + Re-transcribe) and
// PianoRoll (`role=img` canvas wrapper). The scan runs after the panel
// settles into the "complete" branch so progress bars do not race.
//

import { expect, test } from "./fixtures";
import {
  buildSyntheticPolyResult,
  installAnalysisMock,
  installRecordingsMock,
  installTranscribeMock,
  pushTranscribeProgress,
  type MockAnalysisSummary,
  type MockContourResult,
  type MockPolyResult,
  type MockRecording,
  type MockTranscribeSummary,
} from "./helpers/tauri-mock";

const NOW = 1_700_000_000_000;

const SEED: MockRecording[] = [
  {
    id: "rec-tr-a11y-001",
    filename: "axe-transcribe-001.wav",
    createdAt: NOW - 4 * 60 * 1000,
    durationMs: 1_200,
    sampleRateHz: 48000,
    channels: 1,
    bitDepth: 24,
    a4Hz: 440,
    instrumentProfile: "Voice",
  },
];

const SUMMARY: Record<string, MockAnalysisSummary> = {
  "rec-tr-a11y-001": {
    recordingId: "rec-tr-a11y-001",
    medianMidi: 67,
    medianCents: 0,
    voicedRatio: 0.9,
    wasCached: true,
    analyzerVersion: "pyin-0.1.0",
  },
};

const CONTOUR: Record<string, MockContourResult> = {
  "rec-tr-a11y-001:pyin-0.1.0": {
    recordingId: "rec-tr-a11y-001",
    analyzerVersion: "pyin-0.1.0",
    medianMidi: 67,
    medianCents: 0,
    voicedRatio: 0.9,
    frames: [{ tMs: 0, centsFromMedian: 0, voiced: true }],
  },
};

const TRANSCRIBE: Record<string, MockTranscribeSummary> = {
  "rec-tr-a11y-001": {
    recordingId: "rec-tr-a11y-001",
    noteCount: 3,
    durationMs: 1200,
    wasCached: true,
    transcriberVersion: "basicpitch-0.1.0",
  },
};

const POLY: Record<string, MockPolyResult> = {
  "rec-tr-a11y-001:basicpitch-0.1.0": buildSyntheticPolyResult("rec-tr-a11y-001"),
};

test.describe("a11y — transcription regions", () => {
  test("axe scan reports no serious or critical violations", async ({ page, mockTauri, axe }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMock(SUMMARY, CONTOUR),
      ...installTranscribeMock(TRANSCRIBE, POLY),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    // Settle into the complete branch so the panel is stable when axe
    // walks the tree.
    await page.getByTestId("transcribe-button").click();
    await pushTranscribeProgress(page, { recordingId: "rec-tr-a11y-001", percent: 100 });

    await expect(page.getByTestId("transcribe-panel")).toBeVisible();
    await expect(
      page.getByRole("img", { name: /Piano roll: 3 notes between MIDI 64 and 71/i }),
    ).toBeVisible();

    const results = await axe.analyze();
    const blocking = results.violations.filter(
      (v) => v.impact === "serious" || v.impact === "critical",
    );
    expect(blocking, `axe violations:\n${JSON.stringify(blocking, null, 2)}`).toEqual([]);
  });

  test("PianoRoll canvas is aria-hidden — semantic state lives on the figure", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMock(SUMMARY, CONTOUR),
      ...installTranscribeMock(TRANSCRIBE, POLY),
    });
    await page.goto("/");
    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    await page.getByTestId("transcribe-button").click();
    await pushTranscribeProgress(page, { recordingId: "rec-tr-a11y-001", percent: 100 });

    const canvas = page.locator("canvas[data-testid=piano-roll-canvas]");
    await expect(canvas).toHaveAttribute("aria-hidden", "true");

    // The wrapping role=img element carries the composed aria-label per
    // the spec — assert presence here for redundancy with the figure-level
    // tests in piano_roll.spec.ts.
    const figure = page.getByRole("img", { name: /Piano roll/i });
    await expect(figure).toBeVisible();
  });

  test("Transcribe button exposes a button role with the expected label", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMock(SUMMARY, CONTOUR),
      ...installTranscribeMock(TRANSCRIBE, POLY),
    });
    await page.goto("/");
    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    const button = page.getByTestId("transcribe-button");
    await expect(button).toBeVisible();
    const tag = await button.evaluate((el) => el.tagName.toLowerCase());
    const role = await button.getAttribute("role");
    expect(tag === "button" || role === "button").toBe(true);
    await expect(button).toHaveText(/Transcribe to MIDI/i);
  });
});
