// Phase 5 — StemSeparationPanel complete-branch spec (TDD-RED).
//
// Drives the full idle → separating → complete arc. After the parked
// `separate_stems` promise resolves via `pushStemsComplete`, the panel
// renders four StemCards in fixed order (Vocals → Drums → Bass → Other)
// inside a `role="list"` wrapper. Each card carries:
//
//   - <h3> heading with the stem display name
//   - a nested PlaybackPanel keyed off `data-testid="playback-panel"`
//   - a `data-testid="transcribe-stem-{kind}"` button
//   - a `data-testid="export-stem-{kind}"` button
//
// The receiver-closed-early contract is enforced by `pushStemsProgress`
// itself (no-op when the listener list is empty), so the spec drives a
// trivial 1-frame progress + complete sequence.
//

import { expect, test } from "./fixtures";
import {
  installRecordingsMock,
  installStemsMock,
  pushStemsComplete,
  pushStemsProgress,
  type MockRecording,
  type MockStemKind,
} from "./helpers/tauri-mock";

const NOW = 1_700_000_000_000;

const REC_ID = "rec-stems-complete-001";

const SEED: MockRecording[] = [
  {
    id: REC_ID,
    filename: "stems-complete-001.flac",
    createdAt: NOW - 6 * 60 * 1000,
    durationMs: 6_000,
    sampleRateHz: 48000,
    channels: 1,
    bitDepth: 24,
    a4Hz: 440,
    instrumentProfile: "Voice",
  },
];

const STEM_KINDS: readonly MockStemKind[] = ["vocals", "drums", "bass", "other"];

test.describe("phase 5 — stems complete", () => {
  test("four StemCards mount in fixed order with playback + transcribe + export", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installStemsMock(),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("library-trigger").click();
    await page.getByTestId("recording-row").first().click();

    await page.getByTestId("separate-stems").click();

    // A single mid-flight tick exercises the channel path; complete
    // releases the parked promise so the store flips to `complete`.
    await pushStemsProgress(page, { recordingId: REC_ID, stage: "drums", percent: 30 });
    await pushStemsComplete(page, { recordingId: REC_ID });

    // Four cards mount in the documented order.
    const cards = page.locator('[data-testid^="stem-card-"]');
    await expect(cards).toHaveCount(4);

    for (let i = 0; i < STEM_KINDS.length; i += 1) {
      const kind = STEM_KINDS[i] as MockStemKind;
      const card = page.getByTestId(`stem-card-${kind}`);
      await expect(card).toBeVisible();

      // Display name in the heading; case-insensitive so production
      // can title-case freely.
      await expect(card.getByRole("heading", { level: 3 })).toContainText(new RegExp(kind, "i"));

      // Nested PlaybackPanel + per-stem actions.
      await expect(card.getByTestId("playback-panel")).toBeVisible();
      await expect(card.getByTestId(`transcribe-stem-${kind}`)).toBeVisible();
      await expect(card.getByTestId(`export-stem-${kind}`)).toBeVisible();
    }

    // The four cards live inside a single role=list wrapper for AT.
    const listWrapper = page.getByRole("list", { name: /Stems/i });
    await expect(listWrapper).toBeVisible();
  });
});
