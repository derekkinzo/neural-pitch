// Interval drill — full round to completion + history refresh.
//
// `interval_drill.spec.ts` clicks a single choice and stops. This spec
// drives the round-completion chain the single-choice spec leaves
// untouched: answer every prompt → `completeSession` → trainingStore
// history append (localStorage persist) → the landing DrillCard's
// "last attempt" cells refresh.
//
// Completing the tenth prompt clears `currentDrill`, so the Training
// router unmounts the drill and returns to the landing grid automatically
// — no explicit back navigation. The observable outcome is the Intervals
// card flipping from the no-history em-dash to the just-completed
// attempt's accuracy and a "just now" timestamp.
//
// "now" is intentionally NOT pinned: the just-completed attempt records
// `completedAt = Date.now()` and the re-mounted card reads the same wall
// clock, so a sub-30-second round resolves deterministically to
// "just now" without straddling any relative-time bucket boundary.
//

import { expect, test } from "./fixtures";
import {
  buildSyntheticMelody,
  installTrainingMock,
  type MockDrillAttempt,
} from "./helpers/tauri-mock";

const SEED_HISTORY: MockDrillAttempt[] = [];

// 10-prompt session — bounded loop guard so a regression that fails to
// advance the prompt fails fast rather than spinning to the test timeout.
const MAX_CLICKS = 20;

test.describe("interval drill — full round", () => {
  test("answering every prompt completes the session and refreshes the landing card", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installTrainingMock(SEED_HISTORY, buildSyntheticMelody()),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("practice-trigger").click();
    await page.getByTestId("drill-card").nth(0).getByRole("button", { name: /Start/i }).click();
    await expect(page.getByTestId("interval-drill")).toBeVisible();

    const landing = page.getByTestId("training-landing");
    const drill = page.getByTestId("interval-drill");

    // Answer prompts until the drill completes. Each click scores one
    // answer and advances the prompt; the tenth click clears the session
    // and the router routes back to the landing, unmounting the drill.
    for (let i = 0; i < MAX_CLICKS; i += 1) {
      if (await landing.isVisible().catch(() => false)) break;
      // Re-query the first radio each iteration — the grid re-renders per
      // prompt, so a cached handle would go stale.
      await drill.getByRole("radio").first().click();
    }

    // The completed session routed back to the landing on its own.
    await expect(landing).toBeVisible();
    await expect(drill).toHaveCount(0);

    const intervalsCard = page.getByTestId("drill-card").nth(0);
    // Accuracy cell is now a percentage, not the no-history em-dash. The
    // exact figure varies with which choices matched, but the shape is
    // fixed (0%..100%).
    await expect(intervalsCard.getByTestId("drill-card-accuracy")).toHaveText(/^\d+%$/);
    // The attempt completed seconds ago → "just now" relative copy.
    await expect(intervalsCard.getByTestId("drill-card-when")).toHaveText(/just now/i);
  });
});
