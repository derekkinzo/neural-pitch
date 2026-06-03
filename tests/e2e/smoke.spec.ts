// Smoke tests for the Phase-0 placeholder UI.
//
// Confirms:
//   1. The React root mounts and renders the locked Phase-0 strings.
//   2. The mock-Tauri bridge intercepts `invoke('greet', ...)` and the page
//      re-renders with the mocked response.
//
// Cross-references:
//   docs/design/TEST-PLAN.md §6.2 (user flows category)
//   docs/design/DESIGN.md §13.1 (Phase-0 acceptance — no Tauri UI required)

import { test, expect } from "./fixtures";

test.describe("smoke — Phase-0 placeholder", () => {
  test("renders Phase 0 placeholder", async ({ page, mockTauri }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "NeuralPitch" })).toBeVisible();
    await expect(page.getByText("Phase 0 — skeleton")).toBeVisible();
  });

  test("greet command resolves through mock", async ({ page, mockTauri }) => {
    await mockTauri.install({
      greet: "Hello, mock! NeuralPitch core says hi.",
    });
    await page.goto("/");
    await expect(page.locator("pre")).toHaveText("Hello, mock! NeuralPitch core says hi.");
  });
});
