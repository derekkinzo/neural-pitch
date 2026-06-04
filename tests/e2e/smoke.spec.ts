// Smoke tests for the Phase 1.2 live tuner surface.
//
// Confirms:
//   1. The React root mounts and renders the tuner shell.
//   2. The mock-Tauri bridge is wired and `start_capture` resolves through
//      the default mock response (StatusPill → "live").
//
// Cross-references:
//   docs/design/TEST-PLAN.md §6.2 (user flows category)
//   docs/design/DESIGN.md §13.2 (Phase-1 acceptance — live tuner)

import { test, expect } from "./fixtures";

test.describe("smoke — Phase 1.2 tuner shell", () => {
  test("renders tuner shell and meter", async ({ page, mockTauri }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("note-display")).toBeVisible();
    await expect(page.getByRole("meter", { name: /Pitch deviation in cents/i })).toBeVisible();
    await expect(page.getByTestId("settings-trigger")).toBeVisible();
  });

  test("start_capture wires through to status pill", async ({ page, mockTauri }) => {
    await mockTauri.install();
    await page.goto("/");
    const pill = page.getByTestId("status-pill");
    await expect(pill).toHaveAttribute("data-state", "live");
    await expect(page.getByTestId("status-device")).toHaveText("Mock Microphone");
    // Phase-1.3 status-rate cell appends the channel count to the kHz
    // value; integer multiples of 1000 drop the ".0" suffix.
    await expect(page.getByTestId("status-rate")).toHaveText("48 kHz · mono");
  });
});
