// Settings drawer — A4 reference change and downstream effects.
//
// Confirms:
//   1. Settings drawer opens on gear-button click; Escape closes it.
//   2. Changing the A4 numeric input triggers exactly one debounced
//      `configure` invocation per change.
//   3. After A4 is changed to 442, a frame at 442 Hz exact is rendered as
//      "A4" with cents = 0 (i.e. note-format.ts re-targets).
//
// Cross-references:
//   docs/design/TEST-PLAN.md §6.2 (settings flows)
//   docs/design/DESIGN.md §8 (test plan — settings.spec.ts)

import { expect, test } from "./fixtures";
import { getInvokeCalls, makePitchUpdate, pushPitchUpdate } from "./helpers/tauri-mock";

test.describe("settings — A4 reference + debounced configure", () => {
  test("changing A4 fires exactly one configure call", async ({ page, mockTauri }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("settings-trigger").click();
    const drawer = page.getByRole("dialog", { name: /Tuner settings/i });
    await expect(drawer).toBeVisible();

    const a4Input = page.getByTestId("a4-input");
    await a4Input.focus();
    await a4Input.fill("442");

    // Web-first: poll the recorded invoke calls instead of sleeping for
    // longer than the 150 ms debounce. Slow CI machines occasionally
    // exhaust a hard 400 ms wait when paired with a busy worker.
    await expect
      .poll(async () => (await getInvokeCalls(page, "configure")).length, { timeout: 2000 })
      .toBe(1);

    const calls = await getInvokeCalls(page, "configure");
    expect(calls.length).toBe(1);
    const args = calls[0]?.args;
    const settings = (args?.["settings"] ?? {}) as Record<string, unknown>;
    expect(settings["a4_hz"]).toBe(442);
  });

  test("rapid edits within debounce window coalesce to a single call", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("settings-trigger").click();
    const a4Input = page.getByTestId("a4-input");
    await a4Input.focus();

    // Three writes inside the debounce window must collapse to one call.
    // If clearTimeout() is removed from useSettings.ts, each fill below
    // would fire its own configure and this assertion would fail.
    await a4Input.fill("441");
    await a4Input.fill("442");
    await a4Input.fill("443");

    await expect
      .poll(async () => (await getInvokeCalls(page, "configure")).length, { timeout: 2000 })
      .toBe(1);

    const calls = await getInvokeCalls(page, "configure");
    const args = calls[0]?.args;
    const settings = (args?.["settings"] ?? {}) as Record<string, unknown>;
    // Whichever value was the last fill wins.
    expect(settings["a4_hz"]).toBe(443);
  });

  test("smoothing slider dispatches a debounced configure with smoothing_ms", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("settings-trigger").click();
    const slider = page.getByTestId("smoothing-slider");
    await slider.focus();
    // Slider primitive is a native <input type="range"> per `Select`/
    // `Slider` shape; setting via fill() / press() works either way.
    await slider.fill("400");

    await expect
      .poll(async () => (await getInvokeCalls(page, "configure")).length, { timeout: 2000 })
      .toBe(1);

    const readout = page.getByTestId("smoothing-readout");
    await expect(readout).toContainText("400 ms");

    const calls = await getInvokeCalls(page, "configure");
    const args = calls[0]?.args;
    const settings = (args?.["settings"] ?? {}) as Record<string, unknown>;
    expect(settings["smoothing_ms"]).toBe(400);
  });

  test("instrument hint dispatches a debounced configure with instrument_hint", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await page.getByTestId("settings-trigger").click();
    // The instrument hint <select> is the second Select in the drawer
    // (A4 preset is the first). Use accessible-name routing to be robust
    // against re-orderings.
    const select = page.getByLabel(/Instrument hint/i);
    await select.selectOption("Guitar");

    await expect
      .poll(async () => (await getInvokeCalls(page, "configure")).length, { timeout: 2000 })
      .toBe(1);

    const calls = await getInvokeCalls(page, "configure");
    const args = calls[0]?.args;
    const settings = (args?.["settings"] ?? {}) as Record<string, unknown>;
    expect(settings["instrument_hint"]).toBe("Guitar");
  });

  test("A4 change retargets note-format on next frame", async ({ page, mockTauri }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    // Open drawer and bump A4 to 442.
    await page.getByTestId("settings-trigger").click();
    await page.getByTestId("a4-input").fill("442");
    await expect
      .poll(async () => (await getInvokeCalls(page, "configure")).length, { timeout: 2000 })
      .toBe(1);

    // Push a frame at 442 Hz: with a4=442, that's exactly A4 (cents=0).
    await pushPitchUpdate(page, makePitchUpdate({ f0Hz: 442, cents: 0, a4Hz: 442 }));
    await expect(page.getByTestId("note-letter")).toHaveText("A");
    await expect(page.getByTestId("note-octave")).toHaveText("4");
    await expect(page.getByTestId("note-hz")).toContainText("442.00 Hz");
  });

  test("Escape closes drawer", async ({ page, mockTauri }) => {
    await mockTauri.install();
    await page.goto("/");
    await page.getByTestId("settings-trigger").click();
    await expect(page.getByRole("dialog", { name: /Tuner settings/i })).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.getByRole("dialog", { name: /Tuner settings/i })).toHaveCount(0);
  });
});
