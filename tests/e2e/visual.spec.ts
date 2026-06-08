// Visual regression — Phase 1.2 tuner states.
//
// Three canonical states are snapshotted on Chromium-Linux only. The single-OS
// pin is required by Playwright issue #13873 ("Not planned" 2026-05): even
// identical Docker images render subtly differently across host CPU
// architectures, so cross-OS pixel equality is not a viable goal.
//
// All snapshots run with `prefers-reduced-motion: reduce` so HistoryStrip
// renders its static <output> form rather than the canvas spline — keeping
// pixel diffs deterministic regardless of rAF timing.
//

import { test, expect } from "./fixtures";
import {
  buildSyntheticPolyResult,
  installAnalysisMock,
  installAnalysisMockWithRange,
  installPlaybackMock,
  installPlaybackRoutes,
  installRecordingsMock,
  installTranscribeMock,
  makePitchUpdate,
  pushPitchUpdate,
  pushTranscribeProgress,
  type MockAnalysisSummary,
  type MockContourResult,
  type MockPolyResult,
  type MockRangeReport,
  type MockRecording,
  type MockTranscribeSummary,
  type MockVibratoReport,
} from "./helpers/tauri-mock";

test.describe("visual — Phase 1.2 tuner states", () => {
  test.skip(
    ({ browserName }) => browserName !== "chromium",
    "visual baselines pinned to chromium-linux",
  );

  test.beforeEach(async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
  });

  test("silent — no signal", async ({ page, mockTauri }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");
    // Push a non-voiced frame to cement the silent state.
    await pushPitchUpdate(
      page,
      makePitchUpdate({ f0Hz: 0, cents: 0, voiced: false, confidence: 0 }),
    );
    // The NoteDisplay rAF tick should have absorbed the frame; gate on a
    // discriminating positive assertion rather than a fixed sleep so a
    // dropped CI frame does not flake the snapshot.
    await expect(page.getByTestId("note-letter")).toHaveText("—");
    await expect(page).toHaveScreenshot("tuner-silent.png", { fullPage: true });
  });

  test("in-tune — A4 0 cents", async ({ page, mockTauri }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");
    await pushPitchUpdate(page, makePitchUpdate({ f0Hz: 440, cents: 0 }));
    await expect(page.getByRole("meter", { name: /Pitch deviation in cents/i })).toHaveAttribute(
      "data-state",
      "in-tune",
    );
    await expect(page.getByTestId("note-letter")).toHaveText("A");
    await expect(page).toHaveScreenshot("tuner-in-tune.png", { fullPage: true });
  });

  test("sharp — +20 cents", async ({ page, mockTauri }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");
    await pushPitchUpdate(page, makePitchUpdate({ f0Hz: 445, cents: 20 }));
    await expect(page.getByRole("meter", { name: /Pitch deviation in cents/i })).toHaveAttribute(
      "data-state",
      "sharp",
    );
    await expect(page.getByTestId("note-letter")).toHaveText("A");
    await expect(page).toHaveScreenshot("tuner-sharp.png", { fullPage: true });
  });
});

test.describe("visual — Phase 2.1 RecordingDetail", () => {
  test.skip(
    ({ browserName }) => browserName !== "chromium",
    "visual baselines pinned to chromium-linux",
  );

  test.beforeEach(async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
  });

  // Deterministic seed — fixed median, fixed contour samples. Combined
  // with `prefers-reduced-motion: reduce` (synchronous static paint) the
  // snapshot is byte-stable across runs.
  const NOW = 1_700_000_000_000;
  const SEED: MockRecording[] = [
    {
      id: "rec-vis-001",
      filename: "voice-vis-001.flac",
      createdAt: NOW - 5 * 60 * 1000,
      durationMs: 12_000,
      sampleRateHz: 48000,
      channels: 1,
      bitDepth: 24,
      a4Hz: 440,
      instrumentProfile: "Voice",
    },
  ];
  const SUMMARY: Record<string, MockAnalysisSummary> = {
    "rec-vis-001": {
      recordingId: "rec-vis-001",
      medianMidi: 69,
      medianCents: -2.0,
      voicedRatio: 0.9,
      wasCached: true,
      analyzerVersion: "pyin-0.1.0",
    },
  };
  const CONTOUR: Record<string, MockContourResult> = {
    "rec-vis-001:pyin-0.1.0": {
      recordingId: "rec-vis-001",
      analyzerVersion: "pyin-0.1.0",
      medianMidi: 69,
      medianCents: -2.0,
      voicedRatio: 0.9,
      frames: [
        { tMs: 0, centsFromMedian: -8, voiced: true },
        { tMs: 100, centsFromMedian: -4, voiced: true },
        { tMs: 200, centsFromMedian: 0, voiced: true },
        { tMs: 300, centsFromMedian: 3, voiced: true },
        { tMs: 400, centsFromMedian: 6, voiced: true },
        { tMs: 500, centsFromMedian: 0, voiced: false },
        { tMs: 600, centsFromMedian: -2, voiced: true },
        { tMs: 700, centsFromMedian: -5, voiced: true },
        { tMs: 800, centsFromMedian: -7, voiced: true },
        { tMs: 900, centsFromMedian: -3, voiced: true },
        { tMs: 1000, centsFromMedian: 1, voiced: true },
        { tMs: 1100, centsFromMedian: 4, voiced: true },
      ],
    },
  };

  test("recording-detail-cached — header + summary + contour static", async ({
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
    await page.getByTestId("recording-row").first().click();

    // Gate the snapshot on the steady-state Cached badge so a transient
    // progress repaint cannot race the screenshot.
    const summary = page.getByRole("group", { name: /Analysis summary/i });
    await expect(summary).toContainText(/Cached/);
    await expect(page.getByTestId("contour-canvas")).toBeVisible();

    await expect(page).toHaveScreenshot("recording-detail-cached.png", { fullPage: true });
  });

  // Phase 2.3 — RecordingDetail with both new readouts mounted between
  // the summary card and the contour figure. The seeded summary carries
  // both `range` and `vibrato` so the 2-column grid is fully populated.
  test("recording-detail-with-range-vibrato — both readouts steady", async ({
    page,
    mockTauri,
  }) => {
    const RANGE: Record<string, MockRangeReport> = {
      "rec-vis-001": {
        comfortableLowMidi: 60, // C4
        comfortableHighMidi: 77, // F5
        fullLowMidi: 57, // A3
        fullHighMidi: 81, // A5
        voicedFrameCount: 540,
        voiceTypeHints: ["Alto", "Mezzo-soprano"],
      },
    };
    const VIBRATO: Record<string, MockVibratoReport> = {
      "rec-vis-001": {
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
    // Seed both fields in a single summary by composing the two wrappers
    // through the base summary map.
    const summaryWithBoth: Record<
      string,
      MockAnalysisSummary & { range: MockRangeReport; vibrato: MockVibratoReport }
    > = {
      "rec-vis-001": {
        ...SUMMARY["rec-vis-001"]!,
        range: RANGE["rec-vis-001"]!,
        vibrato: VIBRATO["rec-vis-001"]!,
      },
    };
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMockWithRange(
        summaryWithBoth as Record<string, MockAnalysisSummary>,
        CONTOUR,
        RANGE,
      ),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    // Gate on both readout regions being visible before the screenshot
    // so a transient progress repaint cannot race the snapshot.
    await expect(page.getByRole("group", { name: /Vocal range report/i })).toBeVisible();
    await expect(page.getByRole("group", { name: /Vibrato analysis/i })).toBeVisible();
    await expect(page.getByTestId("contour-canvas")).toBeVisible();

    // Use a fullPage screenshot rather than the recording-detail element so
    // the image dimensions are viewport-bounded; element-scoped screenshots
    // varied between Docker and CI runner heights when readout pills wrapped
    // differently.
    await expect(page).toHaveScreenshot("recording-detail-with-range-vibrato.png", {
      fullPage: true,
    });
  });

  // Phase 2.4 — RecordingDetail + PlaybackPanel mounted together. The
  // wavesurfer waveform `<canvas>` is gated on the `ready` event landing
  // before the snapshot. Spectrogram stays hidden so the layout matches
  // the steady-state first-paint case.
  test("recording-detail-with-playback — waveform + transport steady", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMock(SUMMARY, CONTOUR),
      ...installPlaybackMock(),
    });
    await installPlaybackRoutes(page);

    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    // Gate the snapshot on the wavesurfer canvas being mounted (the
    // `ready` event has fired) and the spectrogram being collapsed.
    const panel = page.getByTestId("playback-panel");
    await expect(panel).toBeVisible();
    await expect(panel.locator("canvas").first()).toBeVisible({ timeout: 4000 });
    await expect(page.locator("#spectrogram-host")).toHaveAttribute("hidden", "");

    // Use the project-wide `maxDiffPixelRatio: 0.08` baked into
    // `playwright.config.ts` (no per-test override). The wavesurfer
    // canvas is sensitive to glibc/freetype/cairo drift, so the strict
    // pixel gate the previous revision used did not survive the move
    // from local Playwright runner to the official Microsoft Playwright
    // Docker image. Aligning on the project-wide ratio keeps the visual
    // contract uniform with the other Phase-2 snapshots.
    await expect(page).toHaveScreenshot("recording-detail-with-playback.png", {
      fullPage: true,
    });
  });

  // Phase 3 — RecordingDetail with TranscribePanel + PianoRoll mounted.
  // The piano-roll canvas is painted statically (no rAF loop while
  // paused) so the snapshot is byte-stable across runs once the panel
  // settles into the complete branch.
  test("recording-detail-with-piano-roll — transcribe + canvas steady", async ({
    page,
    mockTauri,
  }) => {
    const TRANSCRIBE: Record<string, MockTranscribeSummary> = {
      "rec-vis-001": {
        recordingId: "rec-vis-001",
        noteCount: 3,
        durationMs: 1200,
        wasCached: true,
        transcriberVersion: "basicpitch-0.1.0",
      },
    };
    const POLY: Record<string, MockPolyResult> = {
      "rec-vis-001:basicpitch-0.1.0": buildSyntheticPolyResult("rec-vis-001"),
    };
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installAnalysisMock(SUMMARY, CONTOUR),
      ...installTranscribeMock(TRANSCRIBE, POLY),
    });

    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    // Run the transcribe button so the panel settles into the complete
    // branch (idle → in-progress → complete) before the screenshot.
    await page.getByTestId("transcribe-button").click();
    await pushTranscribeProgress(page, { recordingId: "rec-vis-001", percent: 100 });

    // Gate on the steady-state piano-roll figure visibility and the
    // contour canvas alongside it so the snapshot does not race a
    // mid-paint frame.
    await expect(
      page.getByRole("img", { name: /Piano roll: 3 notes between MIDI 64 and 71/i }),
    ).toBeVisible();
    await expect(page.getByTestId("contour-canvas")).toBeVisible();

    await expect(page).toHaveScreenshot("recording-detail-with-piano-roll.png", {
      fullPage: true,
    });
  });
});
