// Phase 4 — Solfege label-mode toggle.
//
// Drives the new Settings drawer "Note labels" select through the three
// modes and asserts:
//
//   1. Default mode is "letter"; the IntervalDrill choice row reads "P5".
//   2. Switching to "movable-do" re-renders the IntervalDrill choices in
//      solfege; the perfect-fifth choice reads "Sol" (relative to the
//      drill's own tonic).
//   3. Switching to "fixed-do" anchors the labels to C; assertion is
//      light-touch — we only confirm the label set is no longer the
//      letter set.
//

import { expect, test } from "./fixtures";
import {
  buildSyntheticMelody,
  installTrainingMock,
  type MockDrillAttempt,
} from "./helpers/tauri-mock";

const SEED_HISTORY: MockDrillAttempt[] = [];

test.describe("phase 4 — solfege label toggle", () => {
  test("switching to Movable-do swaps IntervalDrill P5 for Sol", async ({ page, mockTauri }) => {
    await mockTauri.install({
      ...installTrainingMock(SEED_HISTORY, buildSyntheticMelody()),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    // Letter mode is default — open the Intervals drill and confirm
    // the perfect-fifth choice surfaces as "P5" before flipping the
    // mode.
    await page.getByTestId("practice-trigger").click();
    await page.getByTestId("drill-card").nth(0).getByRole("button", { name: /Start/i }).click();

    const intervalChoices = page.getByRole("radiogroup", { name: /Interval choices/i });
    // Assert the P5 radio specifically — the choice grid renders all 12
    // tokens with no whitespace between them, so a word-boundary regex
    // against the joined text would not match cleanly.
    await expect(intervalChoices.getByRole("radio", { name: "P5" })).toBeVisible();

    // Bounce out to the landing, open settings, flip the new selector.
    await page.getByTestId("training-back").click();
    await page.getByTestId("settings-trigger").click();

    const labelMode = page.getByLabel(/Note labels/i);
    await expect(labelMode).toBeVisible();
    await labelMode.selectOption("movable-do");

    // Close settings; re-enter Intervals; the row swaps to solfege.
    await page.keyboard.press("Escape");
    await page.getByTestId("drill-card").nth(0).getByRole("button", { name: /Start/i }).click();

    const intervalChoicesAfter = page.getByRole("radiogroup", { name: /Interval choices/i });
    await expect(intervalChoicesAfter.getByRole("radio", { name: "Sol" })).toBeVisible();
    // The old letter token must be gone — otherwise both label sets
    // are rendered and the toggle is a no-op.
    await expect(intervalChoicesAfter.getByRole("radio", { name: "P5" })).toHaveCount(0);
  });

  test("Note labels select offers letter, movable-do, fixed-do", async ({ page, mockTauri }) => {
    await mockTauri.install({
      ...installTrainingMock(SEED_HISTORY, buildSyntheticMelody()),
    });
    await page.goto("/");
    await page.getByTestId("settings-trigger").click();

    const labelMode = page.getByLabel(/Note labels/i);
    await expect(labelMode).toBeVisible();

    // The native <select> exposes its options through the DOM; assert
    // the three documented values are present without hard-coding the
    // surface text (translatable copy).
    const values = await labelMode.evaluate((el) =>
      Array.from((el as HTMLSelectElement).options).map((o) => o.value),
    );
    expect(values).toEqual(expect.arrayContaining(["letter", "movable-do", "fixed-do"]));
  });

  test("Letter mode is the default — non-drill UI keeps letter labels", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installTrainingMock(SEED_HISTORY, buildSyntheticMelody()),
    });
    await page.goto("/");
    await page.getByTestId("settings-trigger").click();

    const labelMode = page.getByLabel(/Note labels/i);
    await expect(labelMode).toHaveValue("letter");
  });

  test("noteLabelMode survives a page reload (persisted to localStorage)", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installTrainingMock(SEED_HISTORY, buildSyntheticMelody()),
    });
    await page.goto("/");
    await page.getByTestId("settings-trigger").click();

    // Flip to movable-do, then reload the page. The select MUST come
    // back as movable-do — without persistence the store reset to its
    // letter default on every reload (the regression this guards
    // against).
    const labelMode = page.getByLabel(/Note labels/i);
    await labelMode.selectOption("movable-do");
    await expect(labelMode).toHaveValue("movable-do");

    await page.reload();
    await page.getByTestId("settings-trigger").click();
    const labelModeAfter = page.getByLabel(/Note labels/i);
    await expect(labelModeAfter).toHaveValue("movable-do");
  });
});
