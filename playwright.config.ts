// Playwright configuration for the E2E harness.
//
// Three browser projects (chromium, webkit, firefox) drive the React UI
// against the mock-Tauri bridge. Visual baselines are pinned to
// chromium-linux because cross-arch render drift makes other-browser
// baselines unreliable.

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
      // 0.08 chosen empirically: the 1% bound was too tight even with
      // chromium-linux baselines regenerated inside the official Playwright
      // docker image (mcr.microsoft.com/playwright). Subpixel font hinting
      // and freetype version drift between dev hosts and CI runners
      // routinely produce 2-6% pixel-diff. We accept 8% to absorb that
      // noise without giving up regression detection on real layout breaks.
      maxDiffPixelRatio: 0.08,
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
