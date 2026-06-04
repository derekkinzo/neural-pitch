// Visual regression — Phase 1.2 tuner states.
//
// Three canonical states are snapshotted on Chromium-Linux only. The single-OS
// pin is required by Playwright issue #13873 ("Not planned" 2026-05): even
// identical Docker images render subtly differently across host CPU
// architectures, so cross-OS pixel equality is not a viable goal.
//
// All snapshots run with `prefers-reduced-motion: reduce` so HistoryStrip
// renders its static <output> form rather than the canvas spline — keeping
// pixel diffs deterministic regardless of rAF timing.
//
// Cross-references:
//   docs/design/TEST-PLAN.md §6.2 (Visual regression)
//   docs/design/TEST-PLAN.md §11.3 (visual baseline update process)

import { test, expect } from "./fixtures";
import { makePitchUpdate, pushPitchUpdate } from "./helpers/tauri-mock";

test.describe("visual — Phase 1.2 tuner states", () => {
  test.skip(
    ({ browserName }) => browserName !== "chromium",
    "visual baselines pinned to chromium-linux per TEST-PLAN.md §6.2",
  );

  test.beforeEach(async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
  });

  test("silent — no signal", async ({ page, mockTauri }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");
    // Push a non-voiced frame to cement the silent state.
    await pushPitchUpdate(
      page,
      makePitchUpdate({ f0Hz: 0, cents: 0, voiced: false, confidence: 0 }),
    );
    // The NoteDisplay rAF tick should have absorbed the frame; gate on a
    // discriminating positive assertion rather than a fixed sleep so a
    // dropped CI frame does not flake the snapshot.
    await expect(page.getByTestId("note-letter")).toHaveText("—");
    await expect(page).toHaveScreenshot("tuner-silent.png", { fullPage: true });
  });

  test("in-tune — A4 0 cents", async ({ page, mockTauri }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");
    await pushPitchUpdate(page, makePitchUpdate({ f0Hz: 440, cents: 0 }));
    await expect(page.getByRole("meter", { name: /Pitch deviation in cents/i })).toHaveAttribute(
      "data-state",
      "in-tune",
    );
    await expect(page.getByTestId("note-letter")).toHaveText("A");
    await expect(page).toHaveScreenshot("tuner-in-tune.png", { fullPage: true });
  });

  test("sharp — +20 cents", async ({ page, mockTauri }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");
    await pushPitchUpdate(page, makePitchUpdate({ f0Hz: 445, cents: 20 }));
    await expect(page.getByRole("meter", { name: /Pitch deviation in cents/i })).toHaveAttribute(
      "data-state",
      "sharp",
    );
    await expect(page.getByTestId("note-letter")).toHaveText("A");
    await expect(page).toHaveScreenshot("tuner-sharp.png", { fullPage: true });
  });
});
