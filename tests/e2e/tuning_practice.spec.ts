// Tuning-practice drill — completion buttons.
//
// Single-prompt session with two completion affordances:
//   - Done (`tuning-finish-pass`)  → scores the prompt correct.
//   - Skip (`tuning-finish-skip`)  → scores the prompt incorrect.
//
// Both are the only paths to `completeSession` for this drill, and
// `completeSession` clears `currentDrill` — so the Training router
// unmounts the drill and returns to the landing grid. The observable
// difference between the two buttons is the accuracy recorded on the
// Tuning-practice landing card: Done lands 100%, Skip lands 0%.
//
// "now" is intentionally not pinned: the attempt records
// `completedAt = Date.now()` and the re-mounted card reads the same wall
// clock, so the sub-30-second round resolves to "just now".
//

import { expect, test } from "./fixtures";
import {
  buildSyntheticMelody,
  installTrainingMock,
  type MockDrillAttempt,
} from "./helpers/tauri-mock";

const SEED_HISTORY: MockDrillAttempt[] = [];

test.describe("tuning-practice drill — completion buttons", () => {
  test.beforeEach(async ({ page, mockTauri }) => {
    await mockTauri.install({
      ...installTrainingMock(SEED_HISTORY, buildSyntheticMelody()),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("practice-trigger").click();
    // Tuning practice is the fifth card per the documented order.
    await page.getByTestId("drill-card").nth(4).getByRole("button", { name: /Start/i }).click();
    await expect(page.getByTestId("tuning-practice-drill")).toBeVisible();
  });

  test("Done scores the prompt correct — card lands 100%", async ({ page }) => {
    const done = page.getByTestId("tuning-finish-pass");
    await expect(done).toBeVisible();
    await expect(done).toHaveAttribute("aria-label", /Finish tuning practice — pass/i);

    await done.click();

    await expect(page.getByTestId("training-landing")).toBeVisible();
    await expect(page.getByTestId("tuning-practice-drill")).toHaveCount(0);

    const tuningCard = page.getByTestId("drill-card").nth(4);
    await expect(tuningCard.getByTestId("drill-card-accuracy")).toHaveText("100%");
    await expect(tuningCard.getByTestId("drill-card-when")).toHaveText(/just now/i);
  });

  test("Skip scores the prompt incorrect — card lands 0%", async ({ page }) => {
    const skip = page.getByTestId("tuning-finish-skip");
    await expect(skip).toBeVisible();
    await expect(skip).toHaveAttribute("aria-label", /Skip tuning practice/i);

    await skip.click();

    await expect(page.getByTestId("training-landing")).toBeVisible();
    await expect(page.getByTestId("tuning-practice-drill")).toHaveCount(0);

    const tuningCard = page.getByTestId("drill-card").nth(4);
    await expect(tuningCard.getByTestId("drill-card-accuracy")).toHaveText("0%");
    await expect(tuningCard.getByTestId("drill-card-when")).toHaveText(/just now/i);
  });
});
