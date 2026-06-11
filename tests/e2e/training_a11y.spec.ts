// Training accessibility scan.
//
// Mirrors `transcription_a11y.spec.ts` but focused on the ear-training
// drill subsystem. The scan runs in five passes, one per drill screen, so
// any drill-specific axe regression surfaces with a precise pointer.
//
// Contract: zero `serious` / `critical` violations against WCAG 2.1 AA.
//

import { expect, test } from "./fixtures";
import {
  buildSyntheticMelody,
  installTrainingMock,
  type MockDrillAttempt,
} from "./helpers/tauri-mock";

const SEED_HISTORY: MockDrillAttempt[] = [];

const DRILLS = [
  { index: 0, testId: "interval-drill", label: "Intervals" },
  { index: 1, testId: "chord-drill", label: "Chord quality" },
  { index: 2, testId: "scale-drill", label: "Scale ID" },
  { index: 3, testId: "sight-singing-drill", label: "Sight-singing" },
  { index: 4, testId: "tuning-practice-drill", label: "Tuning practice" },
] as const;

test.describe("a11y — training screens", () => {
  test("Training landing reports no serious or critical violations", async ({
    page,
    mockTauri,
    axe,
  }) => {
    await mockTauri.install({
      ...installTrainingMock(SEED_HISTORY, buildSyntheticMelody()),
    });
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("practice-trigger").click();
    await expect(page.getByTestId("training-landing")).toBeVisible();

    const results = await axe.analyze();
    const blocking = results.violations.filter(
      (v) => v.impact === "serious" || v.impact === "critical",
    );
    expect(blocking, `axe violations:\n${JSON.stringify(blocking, null, 2)}`).toEqual([]);
  });

  for (const drill of DRILLS) {
    test(`${drill.label} drill reports no serious or critical violations`, async ({
      page,
      mockTauri,
      axe,
    }) => {
      await mockTauri.install({
        ...installTrainingMock(SEED_HISTORY, buildSyntheticMelody()),
      });
      await page.goto("/");
      await page.getByTestId("practice-trigger").click();

      await page
        .getByTestId("drill-card")
        .nth(drill.index)
        .getByRole("button", { name: /Start/i })
        .click();
      await expect(page.getByTestId(drill.testId)).toBeVisible();

      const results = await axe.analyze();
      const blocking = results.violations.filter(
        (v) => v.impact === "serious" || v.impact === "critical",
      );
      expect(blocking, `axe violations:\n${JSON.stringify(blocking, null, 2)}`).toEqual([]);
    });
  }

  test("Practice trigger exposes a button role with the expected label", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installTrainingMock(SEED_HISTORY, buildSyntheticMelody()),
    });
    await page.goto("/");

    const button = page.getByTestId("practice-trigger");
    await expect(button).toBeVisible();
    const tag = await button.evaluate((el) => el.tagName.toLowerCase());
    const role = await button.getAttribute("role");
    expect(tag === "button" || role === "button").toBe(true);
    await expect(button).toHaveAttribute("aria-label", /ear-training/i);
  });
});
