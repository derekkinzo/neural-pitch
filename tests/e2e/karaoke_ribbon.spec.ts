// Phase 4 — Karaoke ribbon spec.
//
// Drives the SightSingingDrill flow and asserts the KaraokeRibbon paints:
//
//   1. Start the Sight-singing drill from the landing.
//   2. Push 5 synthetic PitchUpdate frames (live ring) AND 5 MatchUpdate
//      frames (per-bar in-tune scoring) through the test bridge.
//   3. The `[data-testid="karaoke-canvas"]` mounts; the wrapping
//      `<figure role="img">` carries an aria-label that recomputes from
//      `liveMatch` and ends with the live cents readout token.
//

import { expect, test } from "./fixtures";
import {
  buildSyntheticMelody,
  installTrainingMock,
  makePitchUpdate,
  pushMatchUpdate,
  pushPitchUpdate,
  type MockDrillAttempt,
  type MockMatchUpdate,
} from "./helpers/tauri-mock";

const SEED_HISTORY: MockDrillAttempt[] = [];

// G3 = MIDI 55 → 196.00 Hz at A4=440. The final MatchUpdate carries
// `current_midi=55` and `cents_offset=-22` so the figure's aria-label
// settles to "current pitch G3 -22 cents".
const FINAL_CENTS = -22;
const FINAL_MIDI = 55;

const MATCH_FRAMES: MockMatchUpdate[] = [
  {
    t_ms: 0,
    target_midi: 60,
    current_midi: 60,
    cents_offset: 4,
    in_tune: true,
    bar_index: 0,
    ended: false,
  },
  {
    t_ms: 250,
    target_midi: 62,
    current_midi: 62,
    cents_offset: -3,
    in_tune: true,
    bar_index: 1,
    ended: false,
  },
  {
    t_ms: 500,
    target_midi: 64,
    current_midi: 63,
    cents_offset: -18,
    in_tune: false,
    bar_index: 2,
    ended: false,
  },
  {
    t_ms: 750,
    target_midi: 65,
    current_midi: 56,
    cents_offset: 30,
    in_tune: false,
    bar_index: 3,
    ended: false,
  },
  {
    t_ms: 1000,
    target_midi: 67,
    current_midi: FINAL_MIDI,
    cents_offset: FINAL_CENTS,
    in_tune: false,
    bar_index: 4,
    ended: true,
  },
];

test.describe("phase 4 — karaoke ribbon", () => {
  test("canvas mounts and figure aria-label ends with live cents readout", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installTrainingMock(SEED_HISTORY, buildSyntheticMelody()),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("practice-trigger").click();
    // Sight-singing is the 4th card per the documented order.
    await page.getByTestId("drill-card").nth(3).getByRole("button", { name: /Start/i }).click();

    await expect(page.getByTestId("sight-singing-drill")).toBeVisible();
    const canvas = page.locator("[data-testid=karaoke-canvas]");
    await expect(canvas).toBeVisible();

    // Drive 5 synthetic live PitchUpdate frames so the moving dot exists.
    const liveFrequencies = [261.63, 293.66, 329.63, 196.0, 196.0];
    for (const f0Hz of liveFrequencies) {
      await pushPitchUpdate(page, makePitchUpdate({ f0Hz, cents: 0 }));
    }

    // Drive 5 MatchUpdate frames so the per-bar scoring colourises bars.
    for (const frame of MATCH_FRAMES) {
      await pushMatchUpdate(page, frame);
    }

    // The figure's aria-label is recomputed off `liveMatch`. After the
    // final frame it ends with the documented suffix — exact match on
    // the trailing cents readout.
    const figure = page.getByRole("img", { name: /Pitch ribbon/i });
    await expect(figure).toBeVisible();
    await expect(figure).toHaveAttribute("aria-label", /current pitch G3 -22 cents$/);
  });

  test("canvas itself is aria-hidden — semantic role lives on the figure", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installTrainingMock(SEED_HISTORY, buildSyntheticMelody()),
    });
    await page.goto("/");
    await page.getByTestId("practice-trigger").click();
    await page.getByTestId("drill-card").nth(3).getByRole("button", { name: /Start/i }).click();

    const canvas = page.locator("[data-testid=karaoke-canvas]");
    await expect(canvas).toBeVisible();
    await expect(canvas).toHaveAttribute("aria-hidden", "true");
  });

  test("prefers-reduced-motion stamps data-reduced-motion on the wrapper", async ({
    page,
    mockTauri,
  }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await mockTauri.install({
      ...installTrainingMock(SEED_HISTORY, buildSyntheticMelody()),
    });
    await page.goto("/");
    await page.getByTestId("practice-trigger").click();
    await page.getByTestId("drill-card").nth(3).getByRole("button", { name: /Start/i }).click();

    const wrapper = page.getByTestId("karaoke-ribbon");
    await expect(wrapper).toBeVisible();
    await expect(wrapper).toHaveAttribute("data-reduced-motion", "true");
  });
});
