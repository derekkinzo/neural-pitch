// Phase 2.0 — recordings drawer accessibility scan.
//
// Asserts that the recordings drawer (open, populated) has zero
// serious or critical axe-core violations against WCAG 2.1 AA. Mirrors
// the existing tuner-a11y baseline in `a11y.spec.ts`.
//

import { expect, test } from "./fixtures";
import { installRecordingsMock, type MockRecording } from "./helpers/tauri-mock";

const NOW = 1_700_000_000_000;

const SEED: MockRecording[] = [
  {
    id: "rec-a11y-001",
    filename: "axe-take-001.flac",
    createdAt: NOW - 10 * 60 * 1000,
    durationMs: 21_000,
    sampleRateHz: 48000,
    channels: 1,
    bitDepth: 24,
    a4Hz: 440,
    instrumentProfile: "Voice",
  },
  {
    id: "rec-a11y-002",
    filename: "axe-take-002.flac",
    createdAt: NOW - 2 * 60 * 1000,
    durationMs: 7_500,
    sampleRateHz: 48000,
    channels: 1,
    bitDepth: 24,
    a4Hz: 440,
    instrumentProfile: "Guitar",
  },
];

test.describe("a11y — recordings drawer", () => {
  test("axe scan reports no serious or critical violations", async ({ page, mockTauri, axe }) => {
    await mockTauri.install({ ...installRecordingsMock(SEED) });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("library-trigger").click();
    const list = page.getByTestId("recordings-list");
    await expect(list).toBeVisible();
    await expect(list.locator("li")).toHaveCount(2);

    const results = await axe.analyze();
    const blocking = results.violations.filter(
      (v) => v.impact === "serious" || v.impact === "critical",
    );
    expect(blocking, `axe violations:\n${JSON.stringify(blocking, null, 2)}`).toEqual([]);
  });

  test("record button exposes a button role with the expected label", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({ ...installRecordingsMock(SEED) });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    // The record button is keyboard-reachable and has an accessible name.
    const button = page.getByTestId("record-button");
    await expect(button).toBeVisible();
    const role = await button.evaluate((el) => el.getAttribute("role") ?? el.tagName.toLowerCase());
    expect(role === "button" || role === "button").toBe(true);

    const label = await button.getAttribute("aria-label");
    expect(label).toMatch(/Start recording/i);
  });
});
