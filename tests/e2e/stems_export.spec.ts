// StemSeparationPanel per-stem action spec.
//
// Drives the StemCard actions that surface after separation completes and
// that no other spec exercises:
//
//   1. "Export FLAC" click → `export_stem` invoke. Asserts the
//      destination-path derivation strips the recording's file extension
//      and appends `-{kind}.flac`, and that the recordingId + stemKind
//      args reach the IPC boundary.
//   2. "Transcribe this stem" click → `transcribe_recording` invoke
//      carrying the per-stem `stemKind` option (a distinct argument shape
//      from the main TranscribePanel, which passes no stemKind).
//   3. The inline per-stem export-error alert when `export_stem` rejects —
//      the only user-visible signal that an export failed.
//
// The complete branch is reached via the parked `separate_stems` promise
// resolved by `pushStemsComplete`, mirroring stems_complete.spec.ts.
//

import { expect, test } from "./fixtures";
import {
  getInvokeCalls,
  installRecordingsMock,
  installStemsMock,
  installTranscribeMock,
  pushStemsComplete,
  pushStemsProgress,
  type MockPolyResult,
  type MockRecording,
  type MockTranscribeSummary,
} from "./helpers/tauri-mock";

const NOW = 1_700_000_000_000;

const REC_ID = "rec-stems-export-001";

const SEED: MockRecording[] = [
  {
    id: REC_ID,
    // A ".flac" extension so the dest-path derivation has something to
    // strip — the safeLabel regex peels the trailing extension.
    filename: "stems-export-001.flac",
    createdAt: NOW - 6 * 60 * 1000,
    durationMs: 6_000,
    sampleRateHz: 48000,
    channels: 1,
    bitDepth: 24,
    a4Hz: 440,
    instrumentProfile: "Voice",
  },
];

// Per-stem transcribe routes through the transcription store, so the
// transcribe IPC surface must be mocked for that branch. The summary seed
// is keyed on the recording id; the store derives `wasCached = !force`.
const TRANSCRIBE: Record<string, MockTranscribeSummary> = {
  [REC_ID]: {
    recordingId: REC_ID,
    noteCount: 2,
    durationMs: 6000,
    wasCached: false,
    transcriberVersion: "basicpitch-0.1.0",
  },
};

const POLY: Record<string, MockPolyResult> = {
  [`${REC_ID}:basicpitch-0.1.0`]: {
    recordingId: REC_ID,
    transcriberVersion: "basicpitch-0.1.0",
    durationMs: 6000,
    notes: [],
  },
};

async function driveToComplete(page: import("@playwright/test").Page): Promise<void> {
  await page.getByTestId("library-trigger").click();
  await page.getByTestId("recording-row").first().click();
  await page.getByTestId("separate-stems").click();
  await pushStemsProgress(page, { recordingId: REC_ID, stage: "vocals", percent: 20 });
  await pushStemsComplete(page, { recordingId: REC_ID });
  await expect(page.getByTestId("stem-card-vocals")).toBeVisible();
}

test.describe("stems export + per-stem actions", () => {
  test("Export FLAC click invokes export_stem with the derived dest path", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installStemsMock(),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await driveToComplete(page);

    expect(await getInvokeCalls(page, "export_stem")).toHaveLength(0);

    await page.getByTestId("export-stem-vocals").click();

    await expect.poll(async () => (await getInvokeCalls(page, "export_stem")).length).toBe(1);
    const calls = await getInvokeCalls(page, "export_stem");
    // The dest path strips the ".flac" extension off the recording label
    // and appends "-vocals.flac"; the recordingId + stemKind args reach
    // the boundary verbatim.
    expect(calls[0]?.args).toMatchObject({
      recordingId: REC_ID,
      stemKind: "vocals",
    });
    expect(String(calls[0]?.args?.["destPath"])).toMatch(/vocals\.flac$/);
    // The derivation must NOT leave the original extension before the
    // stem suffix (i.e. no "stems-export-001.flac-vocals.flac").
    expect(String(calls[0]?.args?.["destPath"])).not.toMatch(/\.flac-vocals\.flac$/);
  });

  test("Transcribe this stem invokes transcribe_recording with stemKind", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installStemsMock(),
      ...installTranscribeMock(TRANSCRIBE, POLY),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await driveToComplete(page);

    const before = (await getInvokeCalls(page, "transcribe_recording")).length;

    await page.getByTestId("transcribe-stem-bass").click();

    // The per-stem transcribe carries a stemKind option — a distinct
    // argument shape from the main TranscribePanel transcribe.
    await expect
      .poll(async () => (await getInvokeCalls(page, "transcribe_recording")).length)
      .toBe(before + 1);
    const calls = await getInvokeCalls(page, "transcribe_recording");
    expect(calls[calls.length - 1]?.args).toMatchObject({
      recordingId: REC_ID,
      stemKind: "bass",
    });
  });

  test("export failure surfaces the inline per-stem alert", async ({ page, mockTauri }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installStemsMock(),
      // Override the export_stem handler so it rejects — the inline
      // export-error alert is the only user-visible failure signal.
      export_stem: () => {
        throw new Error("disk full");
      },
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await driveToComplete(page);

    await page.getByTestId("export-stem-vocals").click();

    const alert = page.getByTestId("stem-export-error-vocals");
    await expect(alert).toBeVisible();
    await expect(alert).toHaveAttribute("role", "alert");
    await expect(alert).toContainText(/Export failed:/i);
    await expect(alert).toContainText(/disk full/i);
  });
});
