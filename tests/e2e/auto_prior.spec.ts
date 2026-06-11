// Auto-prior — StatusPill badge tracks the active search range.
//
// With `instrumentHint = "Generic"` the StatusPill renders the auto-prior
// pill. After a synthetic `PriorNarrowed` event arrives via the
// `audio:backend` channel, the badge text updates to the narrowed range
// and `data-prior-mode` reads "auto".
//

import { test, expect } from "./fixtures";
import { makePitchUpdate, pushDeviceEvent, pushPitchUpdate } from "./helpers/tauri-mock";

test.describe("auto-prior — StatusPill badge", () => {
  test("Generic hint shows Auto badge with narrowed range after PriorNarrowed", async ({
    page,
    mockTauri,
    axe,
  }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    // Default `instrumentHint` is "Generic", so the pre-narrowed badge
    // already reads "Auto · 80–620 Hz" (the FALLBACK_GENERIC range).
    const badge = page.getByTestId("status-prior");
    await expect(badge).toHaveAttribute("data-prior-mode", "auto");
    await expect(badge).toContainText(/Auto · 80–620 Hz/);

    // Push 100 PitchUpdates near 220 Hz — the engine would normally narrow
    // the prior here. Under the mock, the engine is replaced by an explicit
    // PriorNarrowed event below, but the frame stream exercises the rAF
    // path so the FPS budget is also exercised.
    for (let i = 0; i < 100; i += 1) {
      await pushPitchUpdate(page, makePitchUpdate({ f0Hz: 220 + (i % 3), cents: 0 }));
    }

    await pushDeviceEvent(page, { type: "PriorNarrowed", rangeHz: [180, 280] });

    await expect(badge).toHaveAttribute("data-prior-mode", "auto");
    await expect(badge).toContainText(/Auto · 180–280 Hz/);

    // Accessibility scan must remain clean with the new badge in the DOM.
    const results = await axe.analyze();
    const blocking = results.violations.filter(
      (v) => v.impact === "serious" || v.impact === "critical",
    );
    expect(blocking, `axe violations:\n${JSON.stringify(blocking, null, 2)}`).toEqual([]);
  });

  test("explicit hint renders fixed-range badge with lock glyph", async ({
    page,
    mockTauri,
    axe,
  }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    // Open the drawer and switch to "Guitar". The staged range for
    // Guitar is [80, 1300].
    await page.getByTestId("settings-trigger").click();
    await page.getByLabel(/Instrument hint/i).selectOption("Guitar");
    // Close drawer to drop the focus trap before reading the badge.
    await page.keyboard.press("Escape");

    const badge = page.getByTestId("status-prior");
    await expect(badge).toHaveAttribute("data-prior-mode", "explicit");
    await expect(badge).toContainText(/Guitar 80–1300 Hz/);

    const results = await axe.analyze();
    const blocking = results.violations.filter(
      (v) => v.impact === "serious" || v.impact === "critical",
    );
    expect(blocking, `axe violations:\n${JSON.stringify(blocking, null, 2)}`).toEqual([]);
  });
});
