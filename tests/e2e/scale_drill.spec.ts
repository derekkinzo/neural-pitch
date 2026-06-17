// Scale drill spec.
//
// Drives the ScaleDrill flow — a distinct component from IntervalDrill
// with its own church-mode choice set and an ascending sequential synth:
//
//   1. Open Practice; click the Scale-ID card's Start button.
//   2. The drill mounts a `prompt-play` button + a 7-radio choice grid
//      (Ionian / Dorian / Phrygian / Lydian / Mixolydian / Aeolian /
//      Locrian). Clicking `prompt-play` plays the ascending seven-note
//      scale; we observe both the test-hook `audioPlayCount` and an
//      oscillator spy that proves seven notes were scheduled
//      sequentially (not a single note or a no-op).
//   3. Clicking a `scale-choice-{id}` radio scores the answer against the
//      deterministic prompt and surfaces a result toast. The first prompt
//      resolves to Lydian (pickPromptIndex(1) = (1*3) % 7 = 3), so
//      choosing Lydian must read "Correct" and any other mode must read
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

// The deterministic correct mode for the first prompt. ScaleDrill's
// pickPromptIndex(1) = (1 * 3) % 7 = 3, and MODES[3] is Lydian. Driving
// this exact answer discriminates the scoring branch a /Correct|Incorrect/
// regex never could.
const FIRST_PROMPT_CORRECT_ID = "lydian";
const FIRST_PROMPT_WRONG_ID = "ionian";

// Read the cumulative prompt-play counter the drill bumps on every play.
async function readPlayCount(page: Page): Promise<number> {
  return page.evaluate(() => {
    type W = Window & { __neuralPitchTestHooks?: { audioPlayCount?: number } };
    return (window as W).__neuralPitchTestHooks?.audioPlayCount ?? 0;
  });
}

// Read the oscillator-spy counter installed before app boot (below). It
// counts every `AudioContext.prototype.createOscillator` call, so it
// proves the seven scale notes were actually scheduled rather than only
// that the counter-bump line ran.
async function readOscillatorCount(page: Page): Promise<number> {
  return page.evaluate(() => {
    type W = Window & { __neuralPitchOscillatorCount?: number };
    return (window as W).__neuralPitchOscillatorCount ?? 0;
  });
}

test.describe("scale drill", () => {
  test.beforeEach(async ({ page, mockTauri }) => {
    // Spy on createOscillator before any app script runs so the scale
    // synth's real scheduling is observable. An ascending seven-note
    // scale schedules seven oscillators; a broken synth that only bumps
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
    // Scale ID is the third card per the documented order.
    await page.getByTestId("drill-card").nth(2).getByRole("button", { name: /Start/i }).click();
    await expect(page.getByTestId("scale-drill")).toBeVisible();
  });

  test("prompt-play schedules the seven-note scale and advances the play count", async ({
    page,
  }) => {
    const playButton = page.getByTestId("prompt-play");
    await expect(playButton).toBeVisible();
    await expect(playButton).toHaveAttribute("aria-label", /Play scale prompt/i);

    // The drill auto-plays on mount, so the counter is already non-zero;
    // capturing the post-mount baseline lets us prove the CLICK itself
    // drives a fresh play rather than re-asserting the mount play.
    const playsBefore = await readPlayCount(page);
    const oscBefore = await readOscillatorCount(page);

    await playButton.click();

    await expect.poll(() => readPlayCount(page), { timeout: 2000 }).toBe(playsBefore + 1);

    // A church-mode scale is seven notes, so the click must schedule
    // seven oscillators. Asserting the delta proves the full scale was
    // rendered sequentially, distinguishing it from the unconditional
    // counter bump and from a single-note synth.
    await expect
      .poll(() => readOscillatorCount(page), { timeout: 2000 })
      .toBeGreaterThanOrEqual(oscBefore + 7);
  });

  test("choosing the correct mode scores Correct", async ({ page }) => {
    // The first prompt resolves to Lydian; choosing it must read "Correct".
    await page.getByTestId(`scale-choice-${FIRST_PROMPT_CORRECT_ID}`).click();
    const toast = page.getByTestId("drill-result-toast");
    await expect(toast).toBeVisible();
    await expect(toast).toHaveText("Correct");
  });

  test("choosing a wrong mode scores Incorrect", async ({ page }) => {
    // Ionian is wrong for the Lydian first prompt; choosing it must read
    // "Incorrect". Pairing this with the Correct case pins both branches
    // of the modeId === promptMode.id comparison.
    await page.getByTestId(`scale-choice-${FIRST_PROMPT_WRONG_ID}`).click();
    const toast = page.getByTestId("drill-result-toast");
    await expect(toast).toBeVisible();
    await expect(toast).toHaveText("Incorrect");
  });

  test("scale choice grid exposes the seven labelled church modes", async ({ page }) => {
    const radiogroup = page.getByRole("radiogroup", { name: /Scale choices/i });
    await expect(radiogroup).toBeVisible();
    const radios = radiogroup.getByRole("radio");
    // Pin both the count and the labels so a relabel or duplicate-id
    // regression fails here rather than silently passing a count check.
    await expect(radios).toHaveText([
      "Ionian",
      "Dorian",
      "Phrygian",
      "Lydian",
      "Mixolydian",
      "Aeolian",
      "Locrian",
    ]);
  });
});
