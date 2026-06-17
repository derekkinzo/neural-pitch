// Sight-singing drill — completion path.
//
// `karaoke_ribbon.spec.ts` drives the ribbon paint but never finishes the
// drill. This spec exercises the completion affordance the ribbon spec
// leaves untouched.
//
// Completing a drill is a routed action: `handleFinish` scores the single
// prompt correct, calls `completeSession`, and `completeSession` clears
// `currentDrill`. The Training router reads `currentDrill`, so the drill
// unmounts and the user is returned to the landing grid. The observable
// outcome is therefore the landing card refresh, not an in-drill toast —
// the Sight-singing card flips from the no-history em-dash to the
// just-completed attempt's 100% accuracy and a "just now" timestamp.
//
// "now" is intentionally not pinned: the attempt records
// `completedAt = Date.now()` and the re-mounted card reads the same wall
// clock, so the sub-30-second round resolves to "just now" without
// straddling a relative-time bucket boundary.
//

import { expect, test } from "./fixtures";
import {
  buildSyntheticMelody,
  installTrainingMock,
  type MockDrillAttempt,
} from "./helpers/tauri-mock";

const SEED_HISTORY: MockDrillAttempt[] = [];

test.describe("sight-singing drill — finish", () => {
  test.beforeEach(async ({ page, mockTauri }) => {
    await mockTauri.install({
      ...installTrainingMock(SEED_HISTORY, buildSyntheticMelody()),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("practice-trigger").click();
    // Sight-singing is the fourth card per the documented order.
    await page.getByTestId("drill-card").nth(3).getByRole("button", { name: /Start/i }).click();
    await expect(page.getByTestId("sight-singing-drill")).toBeVisible();
  });

  test("Finish completes the session and refreshes the landing card", async ({ page }) => {
    const finish = page.getByTestId("sight-singing-finish");
    await expect(finish).toBeVisible();
    await expect(finish).toHaveAttribute("aria-label", /Finish sight-singing/i);

    await finish.click();

    // Completing the drill routes back to the landing — the drill unmounts.
    await expect(page.getByTestId("training-landing")).toBeVisible();
    await expect(page.getByTestId("sight-singing-drill")).toHaveCount(0);

    // The Sight-singing card now reflects the just-completed attempt:
    // single prompt scored correct → 100%, completed seconds ago.
    const sightCard = page.getByTestId("drill-card").nth(3);
    await expect(sightCard.getByTestId("drill-card-accuracy")).toHaveText("100%");
    await expect(sightCard.getByTestId("drill-card-when")).toHaveText(/just now/i);
  });
});
