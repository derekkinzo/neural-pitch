// Tuner — note-transition and meter-update behaviour.
//
// Drives synthetic PitchUpdates through the mock Channel and asserts that:
//   1. NoteDisplay transitions A4 → C5 when f0 jumps.
//   2. The Hz output reflects the new f0.
//   3. CentsMeter aria-valuenow + data-state track smoothed_cents.
//   4. The CentsMeter shifts to data-state="sharp" / "flat" appropriately.
//
// Cross-references:
//   docs/design/TEST-PLAN.md §6.2 (user-flow specs)
//   docs/design/DESIGN.md §8 (test plan — tuner.spec.ts)

import { expect, test } from "./fixtures";
import { makePitchUpdate, pushPitchUpdate } from "./helpers/tauri-mock";

test.describe("tuner — note transitions", () => {
  test("renders A4 then transitions to C5 on f0 change", async ({ page, mockTauri }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    // A4 = 440 Hz exact, in tune.
    await pushPitchUpdate(page, makePitchUpdate({ f0Hz: 440, cents: 0 }));
    await expect(page.getByTestId("note-letter")).toHaveText("A");
    await expect(page.getByTestId("note-octave")).toHaveText("4");
    await expect(page.getByTestId("note-hz")).toContainText("440.00 Hz");
    const meter = page.getByRole("meter", { name: /Pitch deviation in cents/i });
    await expect(meter).toHaveAttribute("data-state", "in-tune");
    await expect(meter).toHaveAttribute("aria-valuenow", "0");

    // Jump to C5 = 523.25 Hz.
    await pushPitchUpdate(page, makePitchUpdate({ f0Hz: 523.25, cents: 0 }));
    await expect(page.getByTestId("note-letter")).toHaveText("C");
    await expect(page.getByTestId("note-octave")).toHaveText("5");
    await expect(page.getByTestId("note-hz")).toContainText("523");
  });

  test("sharp signal flips meter data-state to sharp", async ({ page, mockTauri }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await pushPitchUpdate(page, makePitchUpdate({ f0Hz: 445, cents: 20 }));
    const meter = page.getByRole("meter", { name: /Pitch deviation in cents/i });
    await expect(meter).toHaveAttribute("data-state", "sharp");
    await expect(meter).toHaveAttribute("aria-valuenow", "20");
  });

  test("flat signal flips meter data-state to flat", async ({ page, mockTauri }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    await pushPitchUpdate(page, makePitchUpdate({ f0Hz: 435, cents: -20 }));
    const meter = page.getByRole("meter", { name: /Pitch deviation in cents/i });
    await expect(meter).toHaveAttribute("data-state", "flat");
    await expect(meter).toHaveAttribute("aria-valuenow", "-20");
  });

  test("unvoiced silence keeps meter in silent state", async ({ page, mockTauri }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    // First push a voiced frame so the meter transitions to a non-silent
    // state. Without this, the silent assertion below would pass even if
    // pushPitchUpdate did nothing — the JSX defaults to data-state="silent".
    await pushPitchUpdate(page, makePitchUpdate({ f0Hz: 440, cents: 0 }));
    const meter = page.getByRole("meter", { name: /Pitch deviation in cents/i });
    await expect(meter).toHaveAttribute("data-state", "in-tune");
    await expect(page.getByTestId("note-letter")).toHaveText("A");

    // Now transition back to silence and assert the discriminating set:
    //   - data-state flips off "in-tune" to "silent"
    //   - the visible note glyph reverts to the dash
    //   - aria-valuetext is the textual silent label
    await pushPitchUpdate(
      page,
      makePitchUpdate({ f0Hz: 0, cents: 0, voiced: false, confidence: 0 }),
    );
    await expect(meter).toHaveAttribute("data-state", "silent");
    await expect(page.getByTestId("note-letter")).toHaveText("—");
    await expect(meter).toHaveAttribute("aria-valuetext", "no signal");
  });
});
