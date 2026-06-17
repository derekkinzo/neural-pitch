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
- `import_button.spec.ts`, `transcribe_button.spec.ts` — file-import and
  polyphonic-transcription entry points.
- `playback_loads.spec.ts`, `playback_spectrogram_toggle.spec.ts`,
  `piano_roll.spec.ts` — recording playback, spectrogram toggle, and
  piano-roll renderer.
- `stems_landing.spec.ts`, `stems_progress.spec.ts`,
  `stems_complete.spec.ts`, `stems_cancel.spec.ts`,
  `stems_export.spec.ts`, `stems_error.spec.ts` —
  HTDemucs stem-separation flow (landing, progress, completion, cancel +
  Escape-to-cancel, per-stem export / transcribe, error branch + Retry).
- `training_landing.spec.ts`, `interval_drill.spec.ts`,
  `karaoke_ribbon.spec.ts`, `solfege_toggle.spec.ts` — ear-training
  drills and the movable-do solfege toggle.
- `a11y.spec.ts`, `recording_a11y.spec.ts`, `recordings_a11y.spec.ts`,
  `range_vibrato_a11y.spec.ts`, `playback_a11y.spec.ts`,
  `stems_a11y.spec.ts`, `training_a11y.spec.ts`,
  `transcription_a11y.spec.ts` — `@axe-core/playwright` scans; fail on
  any `serious` or `critical` WCAG violation.
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

Regenerating `*.png` baselines directly on a developer's host machine
produces bytes that drift from CI because of subpixel font hinting and
freetype version differences (Playwright issue #13873). The supported
flow is to run `scripts/update-visual-baselines.sh`, which executes
Playwright inside the official Playwright Docker image so the rendered
output matches GitHub Actions' `ubuntu-latest` runner:

```sh
scripts/update-visual-baselines.sh
```

The script regenerates the baselines under
`tests/e2e/visual.spec.ts-snapshots/`, then re-runs the visual project
inside the same image to verify the new baselines pass cleanly. Commit
the regenerated PNGs alongside the UI change.

For local exploration only:

```sh
# Local-only; do NOT commit baselines generated this way
npm run e2e:update -- --project=chromium
```

## Mock-Tauri pattern

```ts
import { test, expect } from "./fixtures";

test("status pill goes live on start_capture", async ({ page, mockTauri }) => {
  await mockTauri.install({
    start_capture: () => ({
      device_name: "Mock Microphone",
      sample_rate: 48_000,
      channels: 1,
    }),
  });
  await page.goto("/");
  await expect(page.getByTestId("status-pill")).toHaveAttribute("data-state", "live");
  await expect(page.getByTestId("status-device")).toHaveText("Mock Microphone");
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
