// Playwright configuration for the Tier-5 E2E harness.
//
// Cross-references:
//   docs/design/TEST-PLAN.md §6 (Tier 5 — UI E2E with Playwright MCP)
//   docs/adr/0019-tier-5-e2e-playwright-mcp.md (Track A — Browser-mode every PR)
//
// Three browser projects (chromium, webkit, firefox) gate the Phase-0 placeholder
// surface. Visual baselines are pinned to chromium-linux per TEST-PLAN.md §6.2;
// firefox is wired now so promotion to per-PR coverage is a config flip.

import { defineConfig, devices } from "@playwright/test";

const isCI = Boolean(process.env.CI);

export default defineConfig({
  testDir: "tests/e2e",
  fullyParallel: true,
  forbidOnly: isCI,
  retries: isCI ? 2 : 0,
  ...(isCI ? { workers: 1 } : {}),
  timeout: 30_000,
  expect: {
    toHaveScreenshot: {
      maxDiffPixelRatio: 0.01,
    },
  },
  reporter: [["html", { outputFolder: "playwright-report", open: "never" }], ["github"]],
  use: {
    baseURL: "http://localhost:1420",
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
  projects: [
    {
      name: "chromium",
      use: { ...devices["Desktop Chrome"] },
    },
    {
      name: "webkit",
      use: { ...devices["Desktop Safari"] },
    },
    {
      name: "firefox",
      use: { ...devices["Desktop Firefox"] },
    },
  ],
  webServer: {
    command: "npm run dev",
    url: "http://localhost:1420",
    reuseExistingServer: !isCI,
    timeout: 120_000,
    stdout: "ignore",
    stderr: "pipe",
  },
});
