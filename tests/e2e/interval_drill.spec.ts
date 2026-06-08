// Phase 4 — Interval drill spec.
//
// Drives the IntervalDrill flow:
//
//   1. Open Practice; click the Intervals card's Start button.
//   2. The drill mounts a `prompt-play` button + a 12-radio choice grid
//      (m2..P8). Clicking `prompt-play` synthesises two sine notes through
//      a `new AudioContext()`; we observe the call by counting
//      `__neuralPitchTestHooks.audioPlayCount` (the page-side drill
//      increments on each play).
//   3. Clicking any choice advances the prompt loop; we assert a result
//      toast surfaces with vocabulary "Correct" or "Incorrect".
//

import { expect, test } from "./fixtures";
import {
  buildSyntheticMelody,
  installTrainingMock,
  type MockDrillAttempt,
} from "./helpers/tauri-mock";

const SEED_HISTORY: MockDrillAttempt[] = [];

test.describe("phase 4 — interval drill", () => {
  test.beforeEach(async ({ page, mockTauri }) => {
    await mockTauri.install({
      ...installTrainingMock(SEED_HISTORY, buildSyntheticMelody()),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("practice-trigger").click();
    // Start the Intervals drill — the first card carries the Intervals
    // copy per the documented order.
    await page.getByTestId("drill-card").nth(0).getByRole("button", { name: /Start/i }).click();
    await expect(page.getByTestId("interval-drill")).toBeVisible();
  });

  test("prompt-play synthesises audio and the play count increments", async ({ page }) => {
    const playButton = page.getByTestId("prompt-play");
    await expect(playButton).toBeVisible();

    await playButton.click();

    // The drill increments an audio-play counter on each prompt-play —
    // we use the test hook bag rather than spying on AudioContext so the
    // contract is robust against the underlying synth path (WebAudio
    // node-graph vs. fallback HTMLAudioElement).
    await expect
      .poll(
        async () =>
          page.evaluate(() => {
            type W = Window & {
              __neuralPitchTestHooks?: { audioPlayCount?: number };
            };
            return (window as W).__neuralPitchTestHooks?.audioPlayCount ?? 0;
          }),
        { timeout: 2000 },
      )
      .toBeGreaterThan(0);
  });

  test("interval choice grid exposes 12 radios", async ({ page }) => {
    const radiogroup = page.getByRole("radiogroup", { name: /Interval choices/i });
    await expect(radiogroup).toBeVisible();
    const radios = radiogroup.getByRole("radio");
    await expect(radios).toHaveCount(12);
  });

  test("selecting a choice surfaces a result toast with Correct or Incorrect", async ({ page }) => {
    const radios = page.getByRole("radiogroup", { name: /Interval choices/i }).getByRole("radio");
    // Pick the first choice — the toast contract is independent of which
    // answer the user gives; either path resolves to one of the two
    // result tokens.
    await radios.first().click();

    const toast = page.getByTestId("drill-result-toast");
    await expect(toast).toBeVisible();
    await expect(toast).toHaveText(/Correct|Incorrect/);
  });

  test("ArrowRight cycles focus through radios; Home / End jump to first / last", async ({
    page,
  }) => {
    // Focus the first radio directly — Tab order in the surrounding
    // header changes per drill, so we anchor via the data-testid
    // contract rather than counting Tab presses.
    const firstRadio = page.getByTestId("interval-choice-1");
    await firstRadio.focus();
    await expect(firstRadio).toBeFocused();

    // ArrowRight three times → fourth radio (semitone 4).
    await page.keyboard.press("ArrowRight");
    await page.keyboard.press("ArrowRight");
    await page.keyboard.press("ArrowRight");
    await expect(page.getByTestId("interval-choice-4")).toBeFocused();

    // Home → first radio.
    await page.keyboard.press("Home");
    await expect(firstRadio).toBeFocused();

    // End → last radio (semitone 12).
    await page.keyboard.press("End");
    await expect(page.getByTestId("interval-choice-12")).toBeFocused();

    // ArrowLeft from first → wraps to last; ArrowRight from last → wraps to first.
    await page.keyboard.press("Home");
    await page.keyboard.press("ArrowLeft");
    await expect(page.getByTestId("interval-choice-12")).toBeFocused();
    await page.keyboard.press("ArrowRight");
    await expect(firstRadio).toBeFocused();
  });
});
