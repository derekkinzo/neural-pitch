// Performance budget stub.
//
// Records the intended shape of the perf gate: capture LCP via
// PerformanceObserver and assert against a relaxed CI-friendly bound.
// Skipped until a Web-Vitals target set (e.g. LCP < 2.0 s, CLS < 0.05,
// p95 FPS > 55, no `longtask` > 50 ms) is wired to a real subject.
//

import { test, expect } from "./fixtures";

interface NavigationTimingSubset {
  startTime: number;
  responseEnd: number;
}

test.describe("perf — placeholder stub", () => {
  test.skip(
    true,
    "Tuner UI does not yet expose a real hot path; budgets are defined once one exists to measure.",
  );

  test("LCP under relaxed CI budget", async ({ page, mockTauri }) => {
    await mockTauri.install();
    await page.goto("/");
    await expect(page.getByRole("heading", { name: "NeuralPitch" })).toBeVisible();

    const lcpMs = await page.evaluate<number>(() => {
      const navs = performance.getEntriesByType(
        "navigation",
      ) as unknown as NavigationTimingSubset[];
      if (navs.length === 0) return 0;
      const first = navs[0];
      if (first === undefined) return 0;
      return first.responseEnd - first.startTime;
    });

    expect(lcpMs).toBeLessThan(2500);
  });
});
