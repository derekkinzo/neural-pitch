# Tier-5 E2E Harness — Operator Guide

This directory holds the Playwright-driven Tier-5 UI E2E suite for NeuralPitch. The full plan lives in [`../../docs/design/TEST-PLAN.md`](../../docs/design/TEST-PLAN.md) §6 and the lock is [`../../docs/adr/0019-tier-5-e2e-playwright-mcp.md`](../../docs/adr/0019-tier-5-e2e-playwright-mcp.md).

## What runs here

- `smoke.spec.ts` — Phase-0 placeholder rendering and the mock-Tauri `greet` round-trip.
- `a11y.spec.ts` — `@axe-core/playwright` scan; fails on any `serious` or `critical` WCAG violation.
- `visual.spec.ts` — `toHaveScreenshot` baseline for the placeholder page (Chromium-only; baselines pinned to chromium-linux).
- `i18n.spec.ts` — locale-switching stub, skipped until Phase 4.
- `perf.spec.ts` — Web-Vitals stub, skipped until Phase 1.2.

The mock-Tauri bridge is `helpers/tauri-mock.ts`; it injects `@tauri-apps/api/mocks.mockIPC` via `page.addInitScript` so all `invoke()` calls hit a programmable response map without a real Tauri shell.

## Run locally

```sh
# First-time setup: install browsers + system deps
npx playwright install --with-deps

# Run the full suite (Chromium + WebKit + Firefox)
npm run e2e

# Single project (faster inner loop)
npm run e2e -- --project=chromium

# Single test file
npm run e2e -- tests/e2e/smoke.spec.ts

# Debug UI mode (Playwright's time-travel inspector)
npm run e2e:ui
```

The `webServer` block in `playwright.config.ts` automatically starts `npm run dev` on port 1420 and reuses an already-running dev server outside CI.

## Update visual baselines

Updating `*.png` baselines on a developer's local machine is **not allowed** (Playwright issue #13873 — cross-arch render drift). The supported flow is in [`../../docs/design/TEST-PLAN.md`](../../docs/design/TEST-PLAN.md) §11.3:

1. Push the UI change.
2. CI fails the `visual` spec; the diff PNG is uploaded as an artifact.
3. Comment `/update-snapshots` on the PR.
4. The `update-snapshots` workflow re-runs `npx playwright test --update-snapshots --project=chromium` on `ubuntu-latest` and commits the new baselines.
5. CI re-runs and passes.

For local exploration only:

```sh
# Local-only; do NOT commit baselines generated this way
npm run e2e:update -- --project=chromium
```

## Run cross-browser

```sh
# Chromium + WebKit (per-PR gate)
npm run e2e -- --project=chromium --project=webkit

# Add Firefox (nightly only by policy)
npm run e2e -- --project=firefox
```

## Mock-Tauri pattern

Each spec opens by installing the bridge:

```ts
import { test, expect } from "./fixtures";

test("renders Phase 0 placeholder", async ({ page, mockTauri }) => {
  await mockTauri.install({ greet: "Hello, mock!" });
  await page.goto("/");
  await expect(page.locator("pre")).toHaveText("Hello, mock!");
});
```

`mockTauri.install` accepts a `Record<string, unknown | Function>`. Function values run inside the page (limited to plain serialisable bodies because `addInitScript` structured-clones its arguments). For per-spec overrides that need closures, prefer `page.exposeFunction` plus a small adapter handler.

`pushPitchUpdate(page, frame)` (in `helpers/tauri-mock.ts`) simulates a `Channel<PitchUpdate>` message. It is wired now so Phase-1.2 specs can subscribe via a `usePitchStream` test hook without changing the bridge.

## Reports and artifacts

- HTML report: `playwright-report/` (gitignored).
- Failure traces: `test-results/` (gitignored).
- Screenshots and videos: only on failure (`use.screenshot = "only-on-failure"`, `use.trace = "retain-on-failure"`).

## CI

The `e2e-mock` job in `.github/workflows/ci.yml` runs Chromium + WebKit on every PR; Firefox and the nightly tauri-driver Track-B smoke arrive when Phase 1.2 has real Tauri commands worth driving. See [`../../docs/design/TEST-PLAN.md`](../../docs/design/TEST-PLAN.md) §10.
