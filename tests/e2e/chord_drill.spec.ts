// Chord drill spec.
//
// Drives the ChordDrill flow — a distinct component from IntervalDrill
// with its own chord-quality choice set and a parallel-mode synth:
//
//   1. Open Practice; click the Chord-quality card's Start button.
//   2. The drill mounts a `prompt-play` button + a 7-radio choice grid
//      (Major / Minor / Dim / Aug / Maj7 / Dom7 / Min7). Clicking
//      `prompt-play` triggers the chord notes simultaneously; we observe
//      both the test-hook `audioPlayCount` and an oscillator spy that
//      proves a multi-note chord (not a single note or a no-op) was
//      scheduled in parallel.
//   3. Clicking a `chord-choice-{id}` radio scores the answer against the
//      deterministic prompt and surfaces a result toast. The first prompt
//      resolves to Dom7 (pickPromptIndex(1) = (1*5) % 7 = 5), so choosing
//      Dom7 must read "Correct" and any other quality must read
//      "Incorrect" — pinning both scoring branches.
//

import type { Page } from "@playwright/test";
import { expect, test } from "./fixtures";
import {
  buildSyntheticMelody,
  installTrainingMock,
  type MockDrillAttempt,
} from "./helpers/tauri-mock";

const SEED_HISTORY: MockDrillAttempt[] = [];

// The deterministic correct quality for the first prompt. ChordDrill's
// pickPromptIndex(1) = (1 * 5) % 7 = 5, and CHORDS[5] is the dominant
// seventh. Driving this exact answer discriminates the scoring branch a
// /Correct|Incorrect/ regex never could.
const FIRST_PROMPT_CORRECT_ID = "dom7";
const FIRST_PROMPT_WRONG_ID = "maj";

// Read the cumulative prompt-play counter the drill bumps on every play.
async function readPlayCount(page: Page): Promise<number> {
  return page.evaluate(() => {
    type W = Window & { __neuralPitchTestHooks?: { audioPlayCount?: number } };
    return (window as W).__neuralPitchTestHooks?.audioPlayCount ?? 0;
  });
}

// Read the oscillator-spy counter installed before app boot (below). It
// counts every `AudioContext.prototype.createOscillator` call, so it
// proves notes were actually scheduled rather than only that the
// counter-bump line ran.
async function readOscillatorCount(page: Page): Promise<number> {
  return page.evaluate(() => {
    type W = Window & { __neuralPitchOscillatorCount?: number };
    return (window as W).__neuralPitchOscillatorCount ?? 0;
  });
}

test.describe("chord drill", () => {
  test.beforeEach(async ({ page, mockTauri }) => {
    // Spy on createOscillator before any app script runs so the chord
    // synth's real scheduling is observable. A parallel triad/tetrad
    // schedules one oscillator per note; a broken synth that only bumps
    // the counter schedules none.
    await page.addInitScript(() => {
      type W = Window & {
        __neuralPitchOscillatorCount?: number;
        AudioContext?: typeof AudioContext;
        webkitAudioContext?: typeof AudioContext;
      };
      const w = window as W;
      w.__neuralPitchOscillatorCount = 0;
      const Ctor = w.AudioContext ?? w.webkitAudioContext;
      if (Ctor === undefined) return;
      const original = Ctor.prototype.createOscillator;
      Ctor.prototype.createOscillator = function patched(this: AudioContext) {
        w.__neuralPitchOscillatorCount = (w.__neuralPitchOscillatorCount ?? 0) + 1;
        return original.call(this);
      };
    });

    await mockTauri.install({
      ...installTrainingMock(SEED_HISTORY, buildSyntheticMelody()),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("practice-trigger").click();
    // Chord quality is the second card per the documented order.
    await page.getByTestId("drill-card").nth(1).getByRole("button", { name: /Start/i }).click();
    await expect(page.getByTestId("chord-drill")).toBeVisible();
  });

  test("prompt-play schedules a parallel chord and advances the play count", async ({ page }) => {
    const playButton = page.getByTestId("prompt-play");
    await expect(playButton).toBeVisible();
    await expect(playButton).toHaveAttribute("aria-label", /Play chord prompt/i);

    // The drill auto-plays on mount, so the counter is already non-zero;
    // capturing the post-mount baseline lets us prove the CLICK itself
    // drives a fresh play rather than re-asserting the mount play.
    const playsBefore = await readPlayCount(page);
    const oscBefore = await readOscillatorCount(page);

    await playButton.click();

    await expect.poll(() => readPlayCount(page), { timeout: 2000 }).toBe(playsBefore + 1);

    // The first prompt is Dom7 — a four-note tetrad — so the click must
    // schedule four oscillators in parallel. Asserting the delta proves a
    // real chord was synthesised, distinguishing it from the unconditional
    // counter bump and from a single-note (interval) synth.
    await expect
      .poll(() => readOscillatorCount(page), { timeout: 2000 })
      .toBeGreaterThanOrEqual(oscBefore + 3);
  });

  test("choosing the correct quality scores Correct, a wrong quality scores Incorrect", async ({
    page,
  }) => {
    // The first prompt resolves to Dom7; choosing it must read "Correct".
    await page.getByTestId(`chord-choice-${FIRST_PROMPT_CORRECT_ID}`).click();
    const toast = page.getByTestId("drill-result-toast");
    await expect(toast).toBeVisible();
    await expect(toast).toHaveText("Correct");
  });

  test("choosing a wrong quality scores Incorrect", async ({ page }) => {
    // Major is wrong for the Dom7 first prompt; choosing it must read
    // "Incorrect". Pairing this with the Correct case pins both branches
    // of the chordId === promptChord.id comparison.
    await page.getByTestId(`chord-choice-${FIRST_PROMPT_WRONG_ID}`).click();
    const toast = page.getByTestId("drill-result-toast");
    await expect(toast).toBeVisible();
    await expect(toast).toHaveText("Incorrect");
  });

  test("chord choice grid exposes the seven labelled qualities", async ({ page }) => {
    const radiogroup = page.getByRole("radiogroup", { name: /Chord choices/i });
    await expect(radiogroup).toBeVisible();
    const radios = radiogroup.getByRole("radio");
    // Pin both the count and the labels so a relabel or duplicate-id
    // regression fails here rather than silently passing a count check.
    await expect(radios).toHaveText(["Major", "Minor", "Dim", "Aug", "Maj7", "Dom7", "Min7"]);
  });
});
