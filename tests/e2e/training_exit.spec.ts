// Training — Exit affordance routing.
//
// The `training-back` button (covered by `solfege_toggle.spec.ts`) only
// returns from an active drill to the landing. The `training-exit` button
// is a different action: it flips `tunerStore.view` back to "tuner",
// returning the user to the live-tuner shell. Exit appears on both the
// landing header and the active-drill header.
//
//   1. Landing-header Exit: open Practice, click `training-exit`; the
//      tuner shell (note-display) returns and the training landing is gone.
//   2. Drill-header Exit: enter a drill, click `training-exit`; the tuner
//      shell returns AND the session is aborted (no active drill remains
//      if the user re-enters Training).
//

import { expect, test } from "./fixtures";
import {
  buildSyntheticMelody,
  installTrainingMock,
  type MockDrillAttempt,
} from "./helpers/tauri-mock";

const SEED_HISTORY: MockDrillAttempt[] = [];

test.describe("training — exit to tuner", () => {
  test("landing-header Exit returns to the tuner shell", async ({ page, mockTauri }) => {
    await mockTauri.install({
      ...installTrainingMock(SEED_HISTORY, buildSyntheticMelody()),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("practice-trigger").click();
    await expect(page.getByTestId("training-landing")).toBeVisible();
    // The tuner shell yields to the training screen.
    await expect(page.getByTestId("note-display")).toHaveCount(0);

    await page.getByTestId("training-exit").click();

    // Top-level view routes back to the tuner: note-display returns, the
    // training landing unmounts.
    await expect(page.getByTestId("note-display")).toBeVisible();
    await expect(page.getByTestId("training-landing")).toHaveCount(0);
  });

  test("drill-header Exit returns to the tuner shell and aborts the session", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installTrainingMock(SEED_HISTORY, buildSyntheticMelody()),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("practice-trigger").click();
    // Enter the Intervals drill.
    await page.getByTestId("drill-card").nth(0).getByRole("button", { name: /Start/i }).click();
    await expect(page.getByTestId("interval-drill")).toBeVisible();

    // Exit from the active-drill header.
    await page.getByTestId("training-exit").click();

    // Back in the tuner shell.
    await expect(page.getByTestId("note-display")).toBeVisible();
    await expect(page.getByTestId("interval-drill")).toHaveCount(0);

    // The session was aborted: re-entering Training lands on the drill
    // grid, not back inside the previously-active drill.
    await page.getByTestId("practice-trigger").click();
    await expect(page.getByTestId("training-landing")).toBeVisible();
    await expect(page.getByTestId("interval-drill")).toHaveCount(0);
  });
});
