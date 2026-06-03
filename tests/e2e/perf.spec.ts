// Performance budget stub.
//
// Phase-0 has no real hot path: the placeholder page is a single static
// React tree. Web-Vitals targets defined in TEST-PLAN.md §6.2 (LCP < 2.0 s,
// CLS < 0.05, p95 FPS > 55, no `longtask` > 50 ms) gain teeth in Phase 1.2
// when the tuner needle is the actual measurement subject.
//
// This stub records the intended shape:
//   - capture LargestContentfulPaint via PerformanceObserver
//   - assert against a relaxed CI-friendly bound
//
// Cross-references:
//   docs/design/TEST-PLAN.md §6.2 (Performance category)
//   docs/design/TEST-PLAN.md §11.2 (10% warning band)

import { test, expect } from "./fixtures";

interface NavigationTimingSubset {
  startTime: number;
  responseEnd: number;
}

test.describe("perf — Phase-0 stub", () => {
  test.skip(
    true,
    "Phase 1.2 adds tuner UI; budgets defined when there is a real hot path to measure.",
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
