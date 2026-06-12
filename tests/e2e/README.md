# E2E test harness

Playwright-driven end-to-end suite that drives the Vite dev server with a
mocked Tauri IPC bridge. Each spec installs the bridge via the `mockTauri`
fixture (see `helpers/tauri-mock.ts`) which uses
`@tauri-apps/api/mocks.mockIPC` through `page.addInitScript`, so `invoke()`
calls hit a programmable response map without a real Tauri shell.

## What runs here

- `smoke.spec.ts` — tuner shell mounts and `start_capture` round-trips.
- `tuner.spec.ts`, `auto_prior.spec.ts`, `disconnect.spec.ts`,
  `permission.spec.ts` — live-capture flows.
- `settings.spec.ts` — settings dialog and persistence.
- `recording_lifecycle.spec.ts`, `recording_detail.spec.ts`,
  `recordings_list.spec.ts`, `recording_analysis_cache.spec.ts` —
  recordings library and analysis cache.
- `range_readout.spec.ts`, `vibrato_readout.spec.ts` — range and vibrato
  reports.
- `a11y.spec.ts`, `recording_a11y.spec.ts`, `recordings_a11y.spec.ts`,
  `range_vibrato_a11y.spec.ts` — `@axe-core/playwright` scans; fail on any
  `serious` or `critical` WCAG violation.
- `visual.spec.ts` — `toHaveScreenshot` baseline (Chromium-only;
  `chromium-linux` baselines).
- `i18n.spec.ts`, `perf.spec.ts` — unconditionally skipped pending real
  subjects-of-test (locale switching and a measurable hot path).

## Run locally

```sh
# First-time setup: install browsers + system deps
npx playwright install --with-deps

# Run the full suite
npm run e2e

# Single project (faster inner loop)
npm run e2e -- --project=chromium

# Single test file
npm run e2e -- tests/e2e/smoke.spec.ts

# Debug UI mode
npm run e2e:ui
```

The `webServer` block in `playwright.config.ts` starts `npm run dev` on
port 1420 and reuses an already-running dev server outside CI.

## Update visual baselines

Updating `*.png` baselines on a developer's local machine is not supported
because of cross-arch render drift (Playwright issue #13873). The
supported flow is:

1. Push the UI change.
2. CI fails the `visual` spec; the diff PNG is uploaded as an artifact.
3. Comment `/update-snapshots` on the PR.
4. The `update-snapshots` workflow re-runs
   `npx playwright test --update-snapshots --project=chromium` on
   `ubuntu-latest` and commits the new baselines.
5. CI re-runs and passes.

For local exploration only:

```sh
# Local-only; do NOT commit baselines generated this way
npm run e2e:update -- --project=chromium
```

## Mock-Tauri pattern

```ts
import { test, expect } from "./fixtures";

test("renders tuner", async ({ page, mockTauri }) => {
  await mockTauri.install({ greet: "Hello, mock!" });
  await page.goto("/");
  await expect(page.locator("pre")).toHaveText("Hello, mock!");
});
```

`mockTauri.install` accepts a `Record<string, unknown | Function>`.
Function values run inside the page (limited to plain serialisable bodies
because `addInitScript` structured-clones its arguments). For per-spec
overrides that need closures, prefer `page.exposeFunction` plus a small
adapter handler.

`pushPitchUpdate(page, frame)` (in `helpers/tauri-mock.ts`) simulates a
`Channel<PitchUpdate>` message.

## Reports and artifacts

- HTML report: `playwright-report/` (gitignored).
- Failure traces: `test-results/` (gitignored).
- Screenshots and videos: only on failure
  (`use.screenshot = "only-on-failure"`, `use.trace = "retain-on-failure"`).
