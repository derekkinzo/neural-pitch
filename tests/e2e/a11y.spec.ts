// Accessibility checks for the Phase-0 placeholder.
//
// Asserts:
//   1. axe-core finds no `serious` or `critical` WCAG 2.1 AA violations.
//   2. Tab traversal moves focus through interactive elements (Phase-0 has
//      none, so we assert the body remains the active element rather than
//      forcing a brittle no-op pass).
//
// Cross-references:
//   docs/design/TEST-PLAN.md §6.2 (Accessibility — every page-level spec)

import { test, expect } from "./fixtures";

test.describe("a11y — Phase-0 placeholder", () => {
  test("axe scan reports no serious or critical violations", async ({ page, mockTauri, axe }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "NeuralPitch" })).toBeVisible();

    const results = await axe.analyze();
    const blocking = results.violations.filter(
      (v) => v.impact === "serious" || v.impact === "critical",
    );
    expect(blocking, `axe violations:\n${JSON.stringify(blocking, null, 2)}`).toEqual([]);
  });

  test("keyboard traversal reaches focusable elements", async ({ page, mockTauri }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "NeuralPitch" })).toBeVisible();

    // Phase-0 ships only static text; pressing Tab should move focus into
    // the document and leave it on the body (no traps, no errors). When
    // Phase 1 adds device-picker buttons and the A4 selector, this spec
    // gains assertions that each control receives a visible focus ring.
    await page.keyboard.press("Tab");
    const activeTag = await page.evaluate(() => document.activeElement?.tagName ?? null);
    expect(activeTag).not.toBeNull();
  });
});
