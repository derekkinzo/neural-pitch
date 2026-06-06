# ADR-0019: Tier 5 UI E2E — Playwright MCP browser-mode per-PR + tauri-driver nightly Linux/Windows

## Status

Accepted — 2026-06-03.

Extends [ADR-0016](0016-test-pyramid-tier-1-day-1.md) by adding Tier 5 (UI End-to-End) to the test pyramid. The full mechanics live in [`../design/TEST-PLAN.md`](../design/TEST-PLAN.md).

## Context

Phase 1.2 introduces the React tuner UI riding on the Tauri 2.x shell. The four-tier pyramid in ADR-0016 covers the Rust DSP core thoroughly but is silent on the JS/UI surface — yet the UI is where most user-visible regressions surface (visual changes to the needle, accessibility violations, locale-rendering bugs, broken permissions flow on device disconnect, etc.).

The Tauri 2.x ecosystem in mid-2026 makes UI E2E surprisingly hard:

- **Tauri's WebDriver wrapper, `tauri-driver`, does not support macOS.** Issue tauri-apps/tauri#7068 has been open since 2023-05-26 with no PR activity and the upstream `crates/tauri-driver` README still lists Mac via Appium Mac2 Driver as `[Todo]`. The wrapper ships only for Linux (WebKitGTK via `WebKitWebDriver`) and Windows (Microsoft Edge Driver).
- **Playwright cannot drive a Tauri binary directly.** The protocols are incompatible: Playwright speaks CDP/its own protocol, while macOS WKWebView and Linux WebKitGTK speak W3C WebDriver classic. Microsoft formally declined Tauri support in microsoft/playwright#15404 (closed 2022-07-06).
- **The current `tauri-driver` 2.0.6 + WebdriverIO 9.x combo is broken on stock Ubuntu CI runners.** Tauri issue #15415 (opened 2026-05-18, still open as of 2026-06-03) documents that `tauri-driver` forwards BiDi capabilities verbatim to `WebKitWebDriver`, while WebdriverIO 9.x auto-injects `webSocketUrl: true`, which Ubuntu 22.04/24.04's pre-2.46 `webkit2gtk-driver` rejects. Result: 100% session-start failure with no clean error message. The undocumented workaround is to add `'wdio:enforceWebDriverClassic': true` to every capability block.
- **`tauri-driver` itself has multiple long-standing open bugs** beyond macOS: #6541 (`.click()` / `.setValue()` returning HTTP 500 "unsupported operation" on Linux, open since 2022) and #10670 ("WebDriver e2e testing does not work" docs issue, open). Real-world `tauri-driver` is fragile.
- **Per-PR feedback latency matters.** A native Tauri build cycle plus `tauri-driver` session-start plus xvfb plus Selenium/wdio test execution is several minutes per OS, per PR. That cost doesn't fit the inner loop.

Meanwhile, the productive 2026 alternative is well-trodden: drive the React app in a regular browser via the Vite dev server, with `@tauri-apps/api/mocks` (`mockIPC` + `mockWindows` + `clearMocks` + `shouldMockEvents`, all stable in 2.11.0) injected via `page.addInitScript` _before_ React mounts. Microsoft maintains `@playwright/mcp` (v0.0.75, 2026-05-07, ~33.4k stars) on top of Playwright, so the deterministic suite and the agent-driven exploratory layer share a runtime. At least eight active 2026 production repos (`RandomlyZay-Labs/tauri-app-template`, `Rahuletto/mandy`, `block/sprout`, `owenisas/OmniTool`, `Swofty-Developments/CodeForge`, `jfolcini/agaric`, `hoveychen/claw-fleet`, `ScopeCreep-zip/Rekindle`) ship this exact pattern, confirming it is the de facto industry shape. `block/sprout`'s `window.__SPROUT_E2E__` runtime gate is the production-safety reference.

We need a tier that:

1. Gives per-PR feedback in under two minutes.
2. Covers visual regression, accessibility, user flows, performance, cross-browser, and i18n — not just one of these.
3. Captures shell-level regressions (CSP, IPC wiring, window config) that browser-mode mocking cannot see.
4. Does not depend on tools that don't work on macOS, because much of our development happens on macOS.
5. Pins itself defensively against the documented upstream churn (`tauri-driver` ↔ WebdriverIO 9.x BiDi regression; `@playwright/mcp` cycling ~12 releases/half-year).

## Decision

Add **Tier 5 — UI End-to-End** to the test pyramid, structured as two tracks, all six categories from day one:

- **Track A — Browser-mode Playwright, every PR.** `@playwright/test` 1.60.0 against `vite preview` with `@tauri-apps/api/mocks` injected via `page.addInitScript`. Two browser projects gate every PR (Chromium and WebKit); Firefox runs nightly. Visual regression via `expect(page).toHaveScreenshot()` with baselines pinned to `chromium-linux` only. Accessibility via `@axe-core/playwright`. Performance via `performance.getEntriesByType` + a `requestAnimationFrame` FPS sampler. Cross-browser via Playwright's Chromium and WebKit projects. i18n via `test.use({ locale, timezoneId })` parameterisation.
- **Track B — `tauri-driver` smoke, nightly only.** `tauri-driver` 2.0.6 + WebdriverIO 9.19.x driving the released Tauri binary on `ubuntu-latest` (WebKitGTK via `xvfb-run`) and `windows-latest` (msedgedriver via `chippers/msedgedriver-tool`). Smoke-only: launch, assert React root mounts, run one canonical user flow, assert no console errors, exit. **No macOS** in the nightly matrix.
- **`@playwright/mcp` 0.0.75** is configured under `tests/mcp/` as the agent-driven exploratory layer. It is **not** part of the deterministic CI gate; it is invoked ad hoc by an AI agent or an engineer for exploratory walkthroughs against the same Vite dev server with the same mock bridge enabled.
- **All six categories ship together** rather than being phased: visual regression, accessibility, user flows, performance, cross-browser, i18n. None of them is hard enough on its own to defer, and deferring any of them would lose half the per-PR signal Tier 5 exists to deliver.
- **Defensive pinning** for the duration of the upstream regressions: exact-version pinning of `@playwright/test`, `@playwright/mcp`, `@axe-core/playwright`, `axe-core`, `@wdio/cli`, `tauri-driver`. WebdriverIO capabilities mandatorily include `'wdio:enforceWebDriverClassic': true` until Tauri #15415 is fixed. Re-evaluate quarterly.

The full repo layout, CI workflow YAML, mock-bridge architecture, snapshot baseline-update process, and failure policy are specified in [`../design/TEST-PLAN.md`](../design/TEST-PLAN.md).

## Consequences

- **Per-PR feedback in under 2 min.** Browser-mode against `vite preview` plus mock IPC is fast: a developer pushes, sees Tier 5 results before they've context-switched.
- **macOS developers cannot run the nightly suite locally.** `tauri-driver` does not exist on macOS. macOS coverage is delivered through Track A's WebKit project, which is Playwright's automation-patched build of upstream WebKit main. This is not the same binary as macOS WKWebView, so a release-time manual smoke on real macOS hardware is owed before each tagged release. This is documented in the release checklist.
- **Visual regression has a non-trivial maintenance overhead.** `chromium-linux`-pinned baselines drift any time the UI changes. We expect roughly one baseline-update PR per week of active UI work in Phase 1.5 onward. The CI-artifact-replay update workflow (TEST-PLAN.md §11.3) keeps baselines reproducible, but it does add a step.
- **Nightly tauri-driver flakiness is informational, not blocking.** Tauri #6541 / #10670 / #15415 produce flake we do not own. Three days of consecutive failures on the **same** test is treated as a real regression and triaged.
- **Pinned dependencies require quarterly review.** Playwright MCP cycled through ~12 releases between Feb and May 2026; WebdriverIO 9 BiDi behaviour is a moving target until Tauri #15415 lands a fix. The pin is a tax we accept until upstream stabilises.
- **The mock bridge must never ship to production.** `tests/e2e/mocks/install.ts` is gated behind `import.meta.env.MODE !== 'production'` plus a runtime `window.__E2E__` sentinel, and a `build-hygiene.spec.ts` greps the production bundle for `__E2E_OVERRIDE__` to keep this guarantee. Block/sprout uses the same pattern in production.
- **CI adds two new jobs.** `e2e-mock` (matrix: chromium + webkit) is added to `ci.yml` and becomes a required check on `main`. `e2e-tauri-driver-smoke` (matrix: ubuntu-latest + windows-latest) lives in a new `e2e-nightly.yml` workflow on a 07:00 UTC cron. Branch protection is updated.
- **Storage cost is small.** Audio fixtures (~1 MB), visual baselines (~1 MB), bundled mock script (~50 KB). All committed.

## Alternatives Considered

- **Playwright MCP only (no tauri-driver, ever).** Rejected: misses real-Tauri shell regressions — CSP misconfiguration, broken IPC argument shapes, window-config issues, plugin-manager wiring. The browser-mode mock bridge cannot see these because it runs in a regular Chrome/WebKit process, not under the Tauri binary. The nightly tauri-driver job is a small, mostly-automated way to catch them. Skipping it would let shell-level breakage hit beta users.
- **`tauri-driver` only (drive the Tauri binary every PR).** Rejected: slow (multi-minute Tauri builds plus driver spin-up plus xvfb), unfit for per-PR feedback, fragile (#15415 BiDi/wdio breakage with no upstream fix as of 2026-06-03; #6541 `.click()` returning HTTP 500; #10670 docs issue), and macOS-blocked. macOS being unsupported alone disqualifies it from the gating tier — a substantial fraction of contributor time is on macOS, and a tier they cannot reproduce locally is a tier they will route around.
- **WebdriverIO + `tauri-driver` only (no Playwright at all).** Rejected: smaller ecosystem than Playwright; no MCP server; weaker first-party visual-regression and accessibility tooling; no Vite-dev-server browser-mode story for fast feedback. The official `tauri-apps/webdriver-example` v2 ships WebdriverIO + Mocha as one viable driver, but does not offer the per-PR speed or the agentic-exploration story that Playwright + Playwright MCP do.
- **Selenium + `tauri-driver` only.** Rejected for the same per-PR-speed reasons as the WebdriverIO option, plus a smaller modern test-runner story.
- **Storybook + Chromatic for visual regression.** Rejected: Chromatic OSS tier requires 100+ contributors / 40k weekly downloads / 10k stars (we qualify on none); paid tier starts at $179/month; OSS tier additionally requires public Storybooks, leaking unreleased UI states. Playwright's built-in `toHaveScreenshot` covers our 5-state needle at zero added cost.
- **Percy / Applitools.** Rejected: Percy free tier is the same 5,000 screenshots/month as Argos but with weaker Playwright DX; Applitools has no perpetual free tier and no public pricing. Neither offers anything `toHaveScreenshot` does not, at our scale.
- **Argos-CI from day one.** Deferred (not rejected): Argos's free Hobby tier is genuinely $0/forever for 5,000 screenshots/month with first-class Playwright SDK. Worth re-evaluating once we feel the pain of reviewing PNG diffs in GitHub's diff viewer; not adopted today because we do not yet feel that pain.
- **`jest-image-snapshot` / `looks-same` / standalone `pixelmatch`.** Rejected: duplicate `toHaveScreenshot`'s comparison engine (Playwright already uses `pixelmatch` internally); add a parallel test runner with no review UI; gain nothing.
- **Phase the categories (e.g., visual + flows now, a11y + perf + i18n later).** Rejected: each category is the cheapest to land alongside the rest because they share fixtures, mocks, and helpers. Phasing them defers > 50% of Tier 5's signal for marginal saved engineering.
- **Skip Tier 5 entirely until Phase 2.** Rejected: Phase 1.2 ships the React tuner UI to users; UI regressions in user-visible release builds without a UI test tier is not a defensible posture for a project that already commits to TDD discipline (P4) for the Rust core.
