// TranscribePanel button spec.
//
// Drives the idle → in-progress → complete transition of the new
// TranscribePanel that mounts inside RecordingDetail directly below
// AnalysisSummary:
//
//   1. The seeded recording is selected; the panel renders the primary
//      "Transcribe to MIDI" affordance.
//   2. Clicking it issues `transcribe_recording` and the panel swaps the
//      idle copy for `<progress role="progressbar" aria-label="Transcribing
//      recording">`. We drive `pushTranscribeProgress` ticks at 25/75/100;
//      at 100 the in-flight promise resolves and the bar disappears.
//   3. The complete branch renders "Notes detected: 3" and an "Export
//      MIDI..." affordance plus — because `wasCached=true` from the seed —
//      a "Transcription cached" badge.
//

import { expect, test } from "./fixtures";
import {
  buildSyntheticPolyResult,
  getInvokeCalls,
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
    id: "rec-tr-001",
    filename: "import-take-001.wav",
    createdAt: NOW - 3 * 60 * 1000,
    durationMs: 1_200,
    sampleRateHz: 48000,
    channels: 1,
    bitDepth: 24,
    a4Hz: 440,
    instrumentProfile: "Voice",
  },
];

const SUMMARY: Record<string, MockAnalysisSummary> = {
  "rec-tr-001": {
    recordingId: "rec-tr-001",
    medianMidi: 67,
    medianCents: 0,
    voicedRatio: 0.9,
    wasCached: true,
    analyzerVersion: "pyin-0.1.0",
  },
};

const CONTOUR: Record<string, MockContourResult> = {
  "rec-tr-001:pyin-0.1.0": {
    recordingId: "rec-tr-001",
    analyzerVersion: "pyin-0.1.0",
    medianMidi: 67,
    medianCents: 0,
    voicedRatio: 0.9,
    frames: [
      { tMs: 0, centsFromMedian: 0, voiced: true },
      { tMs: 100, centsFromMedian: 1, voiced: true },
    ],
  },
};

const TRANSCRIBE: Record<string, MockTranscribeSummary> = {
  "rec-tr-001": {
    recordingId: "rec-tr-001",
    noteCount: 3,
    durationMs: 1200,
    wasCached: true,
    transcriberVersion: "basicpitch-0.1.0",
  },
};

const POLY: Record<string, MockPolyResult> = {
  "rec-tr-001:basicpitch-0.1.0": buildSyntheticPolyResult("rec-tr-001"),
};

test.describe("transcribe button", () => {
  test("Transcribe to MIDI swaps in progress bar then renders Notes detected: 3", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMock(SUMMARY, CONTOUR),
      ...installTranscribeMock(TRANSCRIBE, POLY),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    // Idle branch — primary button surfaces below AnalysisSummary.
    const transcribe = page.getByTestId("transcribe-button");
    await expect(transcribe).toBeVisible();
    await expect(transcribe).toHaveText(/Transcribe to MIDI/i);

    await transcribe.click();

    const progress = page.getByRole("progressbar", { name: /Transcribing recording/i });
    await expect(progress).toBeVisible();

    await pushTranscribeProgress(page, { recordingId: "rec-tr-001", percent: 25 });
    await pushTranscribeProgress(page, { recordingId: "rec-tr-001", percent: 75 });
    await pushTranscribeProgress(page, { recordingId: "rec-tr-001", percent: 100 });

    await expect(progress).toBeHidden();

    const panel = page.getByTestId("transcribe-panel");
    await expect(panel).toBeVisible();
    await expect(panel).toContainText(/Notes detected:\s*3/);

    // Export MIDI... affordance surfaces in the complete branch.
    const exportButton = page.getByTestId("export-midi");
    await expect(exportButton).toBeVisible();
    await expect(exportButton).toHaveText(/Export MIDI/i);

    // Exactly one transcribe call was issued for the click (no double-fire
    // from React StrictMode in the test build).
    const calls = await getInvokeCalls(page, "transcribe_recording");
    expect(calls).toHaveLength(1);
    expect(calls[0]?.args).toMatchObject({ recordingId: "rec-tr-001" });
  });

  test("cached badge + Re-transcribe affordance render when wasCached", async ({
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
    await pushTranscribeProgress(page, { recordingId: "rec-tr-001", percent: 100 });

    const panel = page.getByTestId("transcribe-panel");
    await expect(panel).toContainText(/Transcription cached/i);

    const retranscribe = page.getByTestId("retranscribe");
    await expect(retranscribe).toBeVisible();
    await expect(retranscribe).toHaveText(/Re-transcribe/i);

    await retranscribe.click();
    // Forced refresh issues a new call carrying forceRefresh=true.
    await pushTranscribeProgress(page, { recordingId: "rec-tr-001", percent: 100 });
    const calls = await getInvokeCalls(page, "transcribe_recording");
    expect(calls).toHaveLength(2);
    expect(calls[1]?.args).toMatchObject({
      recordingId: "rec-tr-001",
      forceRefresh: true,
    });
  });
});
