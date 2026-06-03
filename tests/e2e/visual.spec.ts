// Visual regression — Phase-0 placeholder snapshot.
//
// One snapshot of the placeholder page on Chromium-Linux only. The single-OS
// pin is required by Playwright issue #13873 ("Not planned" 2026-05): even
// identical Docker images render subtly differently across host CPU
// architectures, so cross-OS pixel equality is not a viable goal.
//
// When Phase 1.2 lands the tuner needle, this spec gains the 5+ canonical
// states (silence, in-tune A4=440, sharp, flat, vibrato, device-disconnected)
// described in TEST-PLAN.md §6.2.
//
// Cross-references:
//   docs/design/TEST-PLAN.md §6.2 (Visual regression)
//   docs/design/TEST-PLAN.md §11.3 (visual baseline update process)

import { test, expect } from "./fixtures";

test.describe("visual — Phase-0 placeholder", () => {
  test.skip(
    ({ browserName }) => browserName !== "chromium",
    "visual baselines pinned to chromium-linux per TEST-PLAN.md §6.2",
  );

  test("tuner-placeholder snapshot", async ({ page, mockTauri }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "NeuralPitch" })).toBeVisible();
    // Phase-0 page is static — no waitForLoadState beyond default.
    await expect(page).toHaveScreenshot("tuner-placeholder.png", {
      fullPage: true,
    });
  });
});
