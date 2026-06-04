// Accessibility checks for the Phase 1.2 tuner.
//
// Asserts:
//   1. axe-core finds no `serious` or `critical` WCAG 2.1 AA violations on
//      the live tuner.
//   2. The settings drawer (when open) also passes the same scan.
//   3. Tab traversal reaches the gear button.
//
// Cross-references:
//   docs/design/TEST-PLAN.md §6.2 (Accessibility — every page-level spec)

import { test, expect } from "./fixtures";

test.describe("a11y — Phase 1.2 tuner", () => {
  test("axe scan reports no serious or critical violations", async ({ page, mockTauri, axe }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("note-display")).toBeVisible();

    const results = await axe.analyze();
    const blocking = results.violations.filter(
      (v) => v.impact === "serious" || v.impact === "critical",
    );
    expect(blocking, `axe violations:\n${JSON.stringify(blocking, null, 2)}`).toEqual([]);
  });

  test("axe scan passes with settings drawer open", async ({ page, mockTauri, axe }) => {
    await mockTauri.install();
    await page.goto("/");
    await page.getByTestId("settings-trigger").click();
    await expect(page.getByRole("dialog", { name: /Tuner settings/i })).toBeVisible();

    const results = await axe.analyze();
    const blocking = results.violations.filter(
      (v) => v.impact === "serious" || v.impact === "critical",
    );
    expect(blocking, `axe violations:\n${JSON.stringify(blocking, null, 2)}`).toEqual([]);
  });

  test("keyboard reaches the settings trigger", async ({ page, mockTauri }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("note-display")).toBeVisible();

    // Tab through until we land on the settings button or run out of patience.
    let landed = false;
    for (let i = 0; i < 10; i += 1) {
      await page.keyboard.press("Tab");
      const focusedTestId = await page.evaluate(
        () => (document.activeElement as HTMLElement | null)?.dataset?.["testid"] ?? null,
      );
      if (focusedTestId === "settings-trigger") {
        landed = true;
        break;
      }
    }
    expect(landed).toBe(true);
  });

  test("drawer traps Tab focus and restores it on close", async ({ page, mockTauri }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("note-display")).toBeVisible();

    // Trigger the drawer through the gear button so focus restoration has
    // a known anchor.
    const trigger = page.getByTestId("settings-trigger");
    await trigger.focus();
    await trigger.click();
    const dialog = page.getByRole("dialog", { name: /Tuner settings/i });
    await expect(dialog).toBeVisible();

    // The first focusable inside the panel receives focus on open (the
    // 0ms setTimeout in Drawer.useEffect). Wait until the activeElement
    // sits inside the dialog.
    await expect
      .poll(async () =>
        page.evaluate(() => {
          const a = document.activeElement as HTMLElement | null;
          return a !== null && a.closest('[role="dialog"]') !== null;
        }),
      )
      .toBe(true);

    // Tab forward through every focusable inside the panel and assert
    // focus never escapes the dialog.
    for (let i = 0; i < 12; i += 1) {
      await page.keyboard.press("Tab");
      const stillInside = await page.evaluate(() => {
        const a = document.activeElement as HTMLElement | null;
        return a !== null && a.closest('[role="dialog"]') !== null;
      });
      expect(stillInside, `focus escaped the dialog on Tab #${i}`).toBe(true);
    }

    // Shift+Tab also stays trapped.
    for (let i = 0; i < 12; i += 1) {
      await page.keyboard.press("Shift+Tab");
      const stillInside = await page.evaluate(() => {
        const a = document.activeElement as HTMLElement | null;
        return a !== null && a.closest('[role="dialog"]') !== null;
      });
      expect(stillInside, `focus escaped the dialog on Shift+Tab #${i}`).toBe(true);
    }

    // Close and assert focus returns to the trigger.
    await page.keyboard.press("Escape");
    await expect(dialog).toHaveCount(0);
    const focusedTestId = await page.evaluate(
      () => (document.activeElement as HTMLElement | null)?.dataset?.["testid"] ?? null,
    );
    expect(focusedTestId).toBe("settings-trigger");
  });
});
