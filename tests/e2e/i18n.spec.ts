// i18n / locale-switching stub.
//
// Phase-0 ships English strings only; the React app does not yet read the
// `?locale=` query param. This stub asserts that the English path renders
// successfully and reserves the spec slot for the Phase-4 movable-do
// solfege NoteFormatter plus the {en-US, de-DE,
// ja-JP, ar-EG} parameterised matrix.
//

import { test, expect } from "./fixtures";

test.describe("i18n — locale switching (stub)", () => {
  test.skip(true, "Phase 4 ear-training adds movable-do; no locale switching UI yet.");

  test("English path renders heading", async ({ page, mockTauri, setLocale }) => {
    await mockTauri.install();
    await setLocale("en-US");
    await expect(page.getByRole("heading", { name: "NeuralPitch" })).toBeVisible();
  });
});
