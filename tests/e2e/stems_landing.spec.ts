// StemSeparationPanel landing spec.
//
// Asserts the idle state of the new StemSeparationPanel that mounts
// inside RecordingDetail as a sibling of TranscribePanel. With no
// previous separation on disk the panel shows a single primary
// "Separate stems" button keyed off `data-testid="separate-stems"`.
//
// The panel is gated by recording selection, so this spec mirrors the
// existing TranscribePanel landing flow:
//   1. Seed one recording via installRecordingsMock.
//   2. Open the library drawer and click the seeded row.
//   3. Assert the stems panel header + idle button render.
//

import { expect, test } from "./fixtures";
import { installRecordingsMock, installStemsMock, type MockRecording } from "./helpers/tauri-mock";

const NOW = 1_700_000_000_000;

const SEED: MockRecording[] = [
  {
    id: "rec-stems-landing-001",
    filename: "stems-landing-001.flac",
    createdAt: NOW - 4 * 60 * 1000,
    durationMs: 4_500,
    sampleRateHz: 48000,
    channels: 1,
    bitDepth: 24,
    a4Hz: 440,
    instrumentProfile: "Voice",
  },
];

test.describe("stems landing", () => {
  test("idle StemSeparationPanel surfaces a Separate stems button", async ({ page, mockTauri }) => {
    await mockTauri.install({
      ...installRecordingsMock(SEED),
      ...installStemsMock(),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("library-trigger").click();
    await expect(page.getByTestId("recordings-list")).toBeVisible();
    await page.getByTestId("recording-row").first().click();

    // Panel itself is keyed; idle button is a stable testid.
    const panel = page.getByTestId("stem-separation-panel");
    await expect(panel).toBeVisible();
    await expect(panel).toContainText(/Stems/);

    const separate = page.getByTestId("separate-stems");
    await expect(separate).toBeVisible();
    await expect(separate).toHaveText(/Separate stems/i);

    // Buttons surface as <button> (or role=button) so AT can reach them
    // — mirrors the practice-trigger contract enforced in training_a11y.
    const tag = await separate.evaluate((el) => el.tagName.toLowerCase());
    const role = await separate.getAttribute("role");
    expect(tag === "button" || role === "button").toBe(true);
  });
});
