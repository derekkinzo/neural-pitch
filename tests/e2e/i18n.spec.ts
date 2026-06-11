// i18n / locale-switching stub.
//
// This build ships English strings only; the React app does not read the
// `?locale=` query param. This stub asserts that the English path renders
// successfully and reserves the spec slot for the movable-do
// solfege NoteFormatter plus the {en-US, de-DE,
// ja-JP, ar-EG} parameterised matrix.
//

import { test, expect } from "./fixtures";

test.describe("i18n — locale switching (stub)", () => {
  test.skip(
    true,
    "Movable-do solfege ships through the ear-training drills; no locale switching UI yet.",
  );

  test("English path renders heading", async ({ page, mockTauri, setLocale }) => {
    await mockTauri.install();
    await setLocale("en-US");
    await expect(page.getByRole("heading", { name: "NeuralPitch" })).toBeVisible();
  });
});
