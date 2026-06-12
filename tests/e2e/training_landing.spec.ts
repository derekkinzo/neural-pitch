// Training landing spec.
//
// Drives the "Practice" header button into the Training screen and asserts
// the five drill cards render with seeded "last attempt" stats:
//
//   1. The Practice trigger lives in the header right-cluster between
//      `library-trigger` and `settings-trigger`. Clicking it flips
//      `tunerStore.view` to `"training"` and the Tuner's main column
//      yields to the Training grid.
//   2. The grid renders five `[data-testid="drill-card"]` cards titled
//      Intervals, Chord quality, Scale ID, Sight-singing, Tuning practice.
//   3. Cards with seeded history render the latest attempt's accuracy
//      and a relative timestamp ("2 days ago"); cards with no history
//      render a single em-dash sentinel ("—").
//
// The drill subsystem is opt-in — we do NOT enter any drill in this spec;
// that contract is covered by `interval_drill.spec.ts`.
//

import { expect, test } from "./fixtures";
import {
  buildSyntheticMelody,
  installTrainingMock,
  type MockDrillAttempt,
} from "./helpers/tauri-mock";

// Pinned epoch: both the spec seed and the page-side render use this
// exact `NOW`. The spec writes it into `__neuralPitchTestHooks.now` via
// `pinTestNow(page, NOW)`; the DrillCard reads the same slot in
// `readTestNow()` and falls back to wall-clock `Date.now()` only in
// production. With both ends pinned to the same epoch the
// `formatRelativeLong` output is exact — no slack required, no flake on
// CI clock drift.
const NOW = 1_700_000_000_000;

// Two seeded attempts: Intervals (84% accuracy, 2 days ago) and Chord
// quality (61% accuracy, 8 hours ago). The other three cards inherit no
// history so the "—" sentinel is asserted alongside the populated cells.
const SEED_HISTORY: MockDrillAttempt[] = [
  {
    id: "att-intervals-001",
    drillId: "intervals",
    // 2 days, 1 minute ago — startedAt → completedAt straddles the
    // 2-day floor cleanly so the rendered copy is exactly "2 days ago".
    startedAt: NOW - (2 * 24 * 60 * 60 * 1000 + 90_000),
    completedAt: NOW - 2 * 24 * 60 * 60 * 1000,
    totalPrompts: 10,
    correctCount: 9,
    accuracy: 0.84,
  },
  {
    id: "att-chords-001",
    drillId: "chords",
    // 8 hours ago — startedAt is 8h+2min ago, completedAt is 8h ago.
    startedAt: NOW - (8 * 60 * 60 * 1000 + 120_000),
    completedAt: NOW - 8 * 60 * 60 * 1000,
    totalPrompts: 10,
    correctCount: 6,
    accuracy: 0.61,
  },
];

/** Pin the page-side `now` so the spec seed and the render share an
 *  epoch — exact `formatRelativeLong` output, no slack required, no
 *  clock-drift flake. */
async function pinTestNow(page: import("@playwright/test").Page, now: number): Promise<void> {
  await page.addInitScript((seedNow: number) => {
    type Hooks = { now?: number; [extra: string]: unknown };
    type WindowWithHooks = Window & { __neuralPitchTestHooks?: Hooks };
    const w = window as WindowWithHooks;
    const hooks: Hooks = w.__neuralPitchTestHooks ?? {};
    hooks.now = seedNow;
    w.__neuralPitchTestHooks = hooks;
  }, now);
}

test.describe("training landing", () => {
  test("Practice button flips view and renders all five drill cards", async ({
    page,
    mockTauri,
  }) => {
    await mockTauri.install({
      ...installTrainingMock(SEED_HISTORY, buildSyntheticMelody()),
    });
    await pinTestNow(page, NOW);
    await page.goto("/");
    await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");

    const practice = page.getByTestId("practice-trigger");
    await expect(practice).toBeVisible();
    await expect(practice).toHaveAttribute("aria-label", /Open ear-training drills/i);

    await practice.click();

    // The Training screen mounts — assert the landing region surfaces.
    await expect(page.getByTestId("training-landing")).toBeVisible();

    const cards = page.getByTestId("drill-card");
    await expect(cards).toHaveCount(5);

    // Titles render in the documented order. We use `nth(...)` so a
    // stable visual order is part of the contract — re-arranging cards
    // is a deliberate action, not a silent UI churn.
    await expect(cards.nth(0)).toContainText(/Intervals/i);
    await expect(cards.nth(1)).toContainText(/Chord quality/i);
    await expect(cards.nth(2)).toContainText(/Scale ID/i);
    await expect(cards.nth(3)).toContainText(/Sight-singing/i);
    await expect(cards.nth(4)).toContainText(/Tuning practice/i);
  });

  test("seeded cards surface latest accuracy + relative timestamp", async ({ page, mockTauri }) => {
    await mockTauri.install({
      ...installTrainingMock(SEED_HISTORY, buildSyntheticMelody()),
    });
    await pinTestNow(page, NOW);
    await page.goto("/");
    await page.getByTestId("practice-trigger").click();

    const intervalsCard = page.getByTestId("drill-card").nth(0);
    await expect(intervalsCard).toContainText(/84\s*%/);
    await expect(intervalsCard).toContainText(/2 days ago/i);

    const chordsCard = page.getByTestId("drill-card").nth(1);
    await expect(chordsCard).toContainText(/61\s*%/);
    await expect(chordsCard).toContainText(/hours? ago/i);
  });

  test("cards without history surface the em-dash sentinel", async ({ page, mockTauri }) => {
    await mockTauri.install({
      ...installTrainingMock(SEED_HISTORY, buildSyntheticMelody()),
    });
    await pinTestNow(page, NOW);
    await page.goto("/");
    await page.getByTestId("practice-trigger").click();

    // Scale ID, Sight-singing, Tuning practice all inherit no history.
    const scaleCard = page.getByTestId("drill-card").nth(2);
    const sightCard = page.getByTestId("drill-card").nth(3);
    const tuningCard = page.getByTestId("drill-card").nth(4);

    await expect(scaleCard).toContainText("—");
    await expect(sightCard).toContainText("—");
    await expect(tuningCard).toContainText("—");
  });

  test("each card exposes a Start affordance", async ({ page, mockTauri }) => {
    await mockTauri.install({
      ...installTrainingMock(SEED_HISTORY, buildSyntheticMelody()),
    });
    await pinTestNow(page, NOW);
    await page.goto("/");
    await page.getByTestId("practice-trigger").click();

    const startButtons = page.getByRole("button", { name: /Start/i });
    // Five drills, five Start buttons.
    await expect(startButtons).toHaveCount(5);
  });
});
