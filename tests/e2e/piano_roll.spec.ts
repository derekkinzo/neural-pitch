// PianoRoll spec.
//
// Asserts the canvas-based piano roll mounts when a `PolyResult` is
// available for the current recording id. Three assertions:
//
//   1. The canvas is wrapped in a `role=img` element whose aria-label
//      embeds the note count and the MIDI range. With the synthetic
//      3-note seed (E4=64, G4=67, B4=71) the label reads
//      "Piano roll: 3 notes between MIDI 64 and 71".
//   2. The canvas has non-zero client size (HiDPI scaling is owned by the
//      shared `lib/canvas-dpr.ts` utility — the spec only checks the
//      logical CSS size).
//   3. `prefers-reduced-motion: reduce` short-circuits auto-scroll: the
//      `[data-reduced-motion]` attribute on the wrapper reads "true" and
//      the canvas paints the static, full-extent view.
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
    id: "rec-pr-001",
    filename: "piano-roll-take-001.wav",
    createdAt: NOW - 2 * 60 * 1000,
    durationMs: 1_200,
    sampleRateHz: 48000,
    channels: 1,
    bitDepth: 24,
    a4Hz: 440,
    instrumentProfile: "Voice",
  },
];

const SUMMARY: Record<string, MockAnalysisSummary> = {
  "rec-pr-001": {
    recordingId: "rec-pr-001",
    medianMidi: 67,
    medianCents: 0,
    voicedRatio: 0.9,
    wasCached: true,
    analyzerVersion: "pyin-0.1.0",
  },
};

const CONTOUR: Record<string, MockContourResult> = {
  "rec-pr-001:pyin-0.1.0": {
    recordingId: "rec-pr-001",
    analyzerVersion: "pyin-0.1.0",
    medianMidi: 67,
    medianCents: 0,
    voicedRatio: 0.9,
    frames: [{ tMs: 0, centsFromMedian: 0, voiced: true }],
  },
};

const TRANSCRIBE: Record<string, MockTranscribeSummary> = {
  "rec-pr-001": {
    recordingId: "rec-pr-001",
    noteCount: 3,
    durationMs: 1200,
    wasCached: true,
    transcriberVersion: "basicpitch-0.1.0",
  },
};

const POLY: Record<string, MockPolyResult> = {
  "rec-pr-001:basicpitch-0.1.0": buildSyntheticPolyResult("rec-pr-001"),
};

test.describe("piano roll", () => {
  test("canvas mounts with role=img and aria-label embedding note count + range", async ({
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

    // The piano roll mounts after a successful transcribe — drive a single
    // 100% progress tick to resolve the in-flight summary call.
    await page.getByTestId("transcribe-button").click();
    await pushTranscribeProgress(page, { recordingId: "rec-pr-001", percent: 100 });

    const figure = page.getByRole("img", {
      name: /Piano roll: 3 notes between MIDI 64 and 71/i,
    });
    await expect(figure).toBeVisible();

    const canvas = page.locator("canvas[data-testid=piano-roll-canvas]");
    await expect(canvas).toBeVisible();
    const dims = await canvas.evaluate((el) => ({
      w: (el as HTMLCanvasElement).clientWidth,
      h: (el as HTMLCanvasElement).clientHeight,
    }));
    expect(dims.w).toBeGreaterThan(0);
    expect(dims.h).toBeGreaterThan(0);
  });

  test("prefers-reduced-motion short-circuits auto-scroll", async ({ page, mockTauri }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMock(SUMMARY, CONTOUR),
      ...installTranscribeMock(TRANSCRIBE, POLY),
    });
    await page.goto("/");
    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    await page.getByTestId("transcribe-button").click();
    await pushTranscribeProgress(page, { recordingId: "rec-pr-001", percent: 100 });

    const wrapper = page.getByTestId("piano-roll");
    await expect(wrapper).toBeVisible();
    await expect(wrapper).toHaveAttribute("data-reduced-motion", "true");
  });

  test("hover surfaces a tooltip with formatted note + timing + velocity", async ({
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
    await pushTranscribeProgress(page, { recordingId: "rec-pr-001", percent: 100 });

    const canvas = page.locator("canvas[data-testid=piano-roll-canvas]");
    await expect(canvas).toBeVisible();

    // Hover the canvas centre — exact coordinates do not matter here, the
    // hit-test logic is component-internal. We only assert that *some*
    // tooltip surfaces with the expected vocabulary fragments.
    await canvas.hover();

    const tooltip = page.getByRole("tooltip");
    await expect(tooltip).toBeVisible();
    await expect(tooltip).toContainText(/E4|G4|B4/);
    await expect(tooltip).toContainText(/vel\s*=\s*\d+/);
  });
});
