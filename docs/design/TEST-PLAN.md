# Test Plan

This document is the addendum to [`DESIGN.md`](DESIGN.md) §10 ("Testing Strategy and TDD Harness"). It expands the four-tier pyramid locked by [ADR-0016](../adr/0016-test-pyramid-tier-1-day-1.md) into a five-tier pyramid by adding **Tier 5 — UI End-to-End** via Playwright + Playwright MCP, locked by [ADR-0019](../adr/0019-tier-5-e2e-playwright-mcp.md). It also concretizes Tier 2 / Tier 3 / Tier 4 to the file paths, scripts, datasets, and CI jobs that will exist in the repo.

DESIGN.md §10 remains canonical for tiers 1–4; this document supersedes it only for Tier 5 (E2E), and refines the per-tier mechanics with what we now know after Phase 0 has shipped.

## Status

Accepted — 2026-06-03. Tier 1 is shipped. Tier 5 (browser-mode) is the active workstream of Phase 1.5. Tiers 2/3/4 land in Phase 1.4 / Phase 2 / release respectively, on the schedule already defined in ADR-0016.

## 1. Five-Tier Pyramid

```
                                 ▲ slow / expensive / few
                                 │
                          ┌──────┴──────┐
                          │   Tier 4    │   MAESTRO + MUSDB18-HQ benchmarks (release-time)
                          ├─────────────┤
                          │   Tier 3    │   MDB-stem-synth + GuitarSet + Bach10 slices (--features dataset)
                          ├─────────────┤
                          │   Tier 5    │   UI E2E: Playwright (per-PR) + tauri-driver nightly
                          ├─────────────┤
                          │   Tier 2    │   Philharmonia single-note voice fixtures (committed)
                          ├─────────────┤
                          │   Tier 1    │   Synthesized signals + proptest + golden tables
                          └─────────────┘
                                 │
                                 ▼ fast / cheap / many
```

Tier 5 sits between Tier 2 and Tier 3 in the pyramid because, like Tier 2, it gates **every PR** with seconds-not-minutes feedback; unlike Tier 2 it exercises the React UI rather than the Rust DSP core. Tier 5's nightly tauri-driver smoke job is order-of-magnitude slower (several minutes per OS) and so is gated like Tier 3 — out of the per-PR critical path.

| Tier | Scope                                                 | Source                                                            | Phase | Gating                                                        |
| ---- | ----------------------------------------------------- | ----------------------------------------------------------------- | ----- | ------------------------------------------------------------- |
| 1    | Rust core unit + property tests                       | Synthesized signals; `proptest`; MIDI 0–127 golden table          | 0     | Every `cargo test`; CI required                               |
| 2    | Rust integration on real audio fragments              | `tests/fixtures/philharmonia/*.flac` (committed, ~MBs)            | 1.4   | Every PR (cargo test, no feature flag)                        |
| 3    | Rust integration on dataset slices                    | `tests/data/{mdb-stem-synth,guitarset,bach10}` (gitignored)       | 2     | `cargo test --features dataset`; nightly + manual; not per-PR |
| 4    | Full benchmark suites                                 | MAESTRO v3, MUSDB18-HQ                                            | rel.  | Manual / self-hosted; results recorded in CHANGELOG           |
| 5    | UI E2E: visual / a11y / flows / perf / x-browser/i18n | React app via Vite dev server (mock-Tauri) + nightly tauri-driver | 1.5   | Browser-mode every PR; tauri-driver nightly Linux + Windows   |

## 2. Tier 1 — Synthesized Signals (status: shipped Phase 0)

Tier 1 is the fast inner loop. It runs in seconds, on every `cargo test`, on every push, and on every PR across {Linux, macOS, Windows} × {stable, beta}. It is the only tier that gates merges in Phase 0.

What Tier 1 covers today, in `crates/neural-pitch-core/tests/`:

- **`golden_note_table.rs`** — `frequency_to_note` round-trip across MIDI 0–127 against a frozen reference table. Catches off-by-one cents bugs and A4-anchor regressions (ADR-0005).
- **`property_tests.rs`** — `proptest` invariants on Hann/Hamming/Blackman windows (sum > 0, monotone-symmetric, energy bounds), framing math (frame count = 1 + (n - frame) / hop), and frequency↔MIDI inversion (`freq → midi → freq` round-trips to within 1 cent for clamped inputs).
- **`smoothing.rs`** — exponential / median smoothing invariants: monotone trajectories produce monotone outputs; constant inputs produce constant outputs after warmup; spikes are attenuated by the configured factor.
- **`voicing.rs`** — voiced/unvoiced classifier on synthesized sine vs noise vs silence; RMS gating threshold round-trip.
- **`yin_property.rs`** — YIN difference-function invariants: `d(0) = 0`, `d(τ)` non-negative, parabolic-interpolation refinement reduces error vs raw integer-`τ` on synthesized sines.
- **`yin_smoke.rs`** — YIN end-to-end on canonical synthesized signals: sine at 440 Hz, sine at 82.41 Hz (E2 cello bottom), vibrato (5 Hz / ±20 cent), two-tone (440 + 880, octave-error trap), white-noise rejection, silence rejection.

Tier 1 also includes the `tests/HARDWARE_RIG.md` spec (ADR-0016 §"Hardware-sanity check") that documents the latency-measurement rig used to gate the Phase-1 acceptance bound (p50 ≤ 45 ms, p99 ≤ 70 ms). The rig itself is human-driven, not CI-driven.

Tests are exempt from `unwrap_used`/`expect_used` lints (ADR-0018 §"Workspace lints" and DESIGN.md §2.2): in tests we want loud failures, not quiet error-conversion noise.

## 3. Tier 2 — Philharmonia Fixtures (Phase 1.4)

**Goal.** Catch regressions on real-instrument audio that synthesized signals will never produce: breath noise on flute attacks, bow noise on cello, sustain decay on plucked strings, finger-squeak on guitar.

**What we ship.** A small, hand-curated subset of the Philharmonia Orchestra single-note sample library, committed to `crates/neural-pitch-core/tests/fixtures/philharmonia/`:

- One `.flac` per pitch class × instrument × dynamic for ~5 instruments (flute, violin, cello, guitar, voice-soprano), at quarter-note duration. Total budget: < 10 MB, < 50 files.
- A `MANIFEST.toml` listing each file with expected fundamental Hz, expected MIDI note, source attribution, and license tag.
- A `LICENSE-PHILHARMONIA.md` capturing the Philharmonia samples' Creative Commons license terms (CC-BY-NC-SA 3.0 unported as published by Philharmonia Orchestra; we keep them under their original license alongside our dual MIT/Apache-2.0 — ADR-0001 — by treating them as test data, not redistributable artifact).

**Where they go.** `crates/neural-pitch-core/tests/fixtures/philharmonia/` (committed). Files are decoded via `symphonia` (already a workspace dep through `cpal`'s indirect graph in Phase 1.1) into `Vec<f32>` mono at 48 kHz, then fed to the same `PitchEstimator` trait used in Tier 1.

**Integration-test pattern.** Each test file in `crates/neural-pitch-core/tests/philharmonia_*.rs` iterates `MANIFEST.toml`, decodes each `.flac`, runs YIN/MPM, and asserts:

- detected fundamental within ±5 cents of manifest expected (tighter on sustained portion, wider on attack transient)
- voiced flag = true on the sustained portion, false during silence padding
- no octave errors (detected octave matches manifest)

Failures dump the offending file path + expected/actual into the panic message; no analysis cache is consulted (Tier 2 is end-to-end-fresh by design).

**Gating.** Every PR. No feature flag. Runs as part of the existing `cargo test --workspace --all-features` step in `ci.yml` § `test`.

**Pre-merge cost.** ~10 s added to the per-PR test matrix (Linux/macOS/Windows × stable/beta = 6 cells). Total CI wall-clock impact: ~60 s aggregated, parallelisable. Acceptable.

## 4. Tier 3 — Dataset Slices (Phase 2 boundary)

**Goal.** Validate that the algorithm behaves correctly on academically-curated datasets with ground-truth annotations. This is where neural-backend regressions (Phase 2, ADR-0008) get caught against published benchmarks.

**Datasets.**

- **MDB-stem-synth** — 230 monophonic stems with perfect synthetic ground-truth f0; the standard reference for monophonic pitch tracking.
- **GuitarSet** — 360 short solo-guitar excerpts with hexaphonic-pickup ground truth; tests guitar-specific failure modes (string-coupling, bend-tracking).
- **Bach10** — 10 J.S. Bach four-part chorales rendered by 4 instruments (violin, clarinet, saxophone, bassoon) with per-source f0; tests polyphonic decomposition and the Phase-2 multi-pitch story.

**Download script.** `scripts/fetch-test-data.sh` downloads each dataset to `tests/data/<dataset>/`, verifies SHA-256 sums against `scripts/fetch-test-data.sums`, and writes a `.fetched` marker. The script is idempotent. `tests/data/` is gitignored (DESIGN.md ADR-0016 Tier 3 row). License terms and citations are surfaced in `tests/data/README.md`.

**Cargo gate.** `cargo test --features dataset`. The `dataset` feature is already declared on `neural-pitch-core` (`Cargo.toml [features]`). Tier-3 test files use `#[cfg(feature = "dataset")]` and read fixtures via a `tests/data/` path resolved relative to `CARGO_MANIFEST_DIR`. Without the feature flag, the tests are absent from the binary.

**Gating.** Not on every PR. Triggered by:

1. Manual `cargo test --features dataset` on a developer's machine.
2. A nightly GitHub Actions workflow that downloads, caches, and runs `cargo test --features dataset` once per day on Linux only, posting results to the actions summary.
3. Release-candidate PRs (the release manager runs Tier 3 manually before tagging).

**Why not per-PR.** Datasets are hundreds of MB to several GB. Bandwidth and runtime exceed the per-PR budget. Caching the GitHub Actions cache helps but is not free.

## 5. Tier 4 — Full Benchmarks (deferred to release)

**Goal.** Reproduce published benchmark numbers on the neural backends to anchor our claims.

**Benchmarks.**

- **MAESTRO v3** — ~200 GB of paired audio + MIDI for piano transcription. Used to validate the Phase-2 transcription path (ADR-0009 phase ordering).
- **MUSDB18-HQ** — ~30 GB stereo at 44.1/48 kHz; standard for source-separation evaluation.

**Run profile.** Self-hosted hardware or a developer-local run. Not on hosted CI: hosted runners cannot reasonably download or compute these in a useful timeframe, and storage egress for the artifacts is prohibitive.

**Acceptance.** Each release records the measured numbers in `CHANGELOG.md` so that regressions across releases are visible. Numbers are reported with hardware fingerprint (CPU, GPU if used, OS, RAM) so that they are reproducible.

**Why deferred to release.** ADR-0016 already locks this; Tier 4 is "manual; results recorded in CHANGELOG." Phase 1 ships without neural backends, so MAESTRO/MUSDB are not yet a meaningful test target.

## 6. Tier 5 — UI E2E with Playwright MCP (NEW)

This is the new tier. Locked by [ADR-0019](../adr/0019-tier-5-e2e-playwright-mcp.md).

### 6.1 Two tracks

- **Track A — Browser-mode every PR.** `@playwright/test` against `vite preview` (or `vite dev` locally) with `@tauri-apps/api/mocks.mockIPC` injected via `page.addInitScript` before React mounts. Three browser projects (Chromium, WebKit, Firefox), with the latter two as cross-engine guardrails. Runs in under 2 minutes per PR.
- **Track B — Nightly tauri-driver smoke.** `tauri-driver` 2.0.6 driving the actually-built Tauri 2.x binary on `ubuntu-latest` (WebKitGTK via `xvfb-run`) and `windows-latest` (msedgedriver). Smoke-only: launches the app, asserts the React root mounts, exercises one canonical user flow, asserts no console errors, exits clean. **No macOS coverage** — `tauri-driver` does not support macOS WKWebView and the upstream issue (`tauri-apps/tauri#7068`) has been open since 2023 with no PR activity. macOS coverage is delivered through Track A's WebKit project, which is the closest cross-platform analog to WKWebView (Playwright's WebKit is upstream-WebKit-main with automation patches; not the same binary as macOS WKWebView, but the closest viable approximation).

### 6.2 Six categories — all in scope from day one

ADR-0019 mandates that **all six** standard E2E categories ship together rather than being phased. Each is concretely landable in Track A; tauri-driver in Track B contributes signal for shell-level regressions only.

1. **Visual regression.** `expect(page).toHaveScreenshot()` for the tuner needle in 5+ canonical states (silence, in-tune A4=440, sharp, flat, vibrato, device-disconnected, optionally other-A4). Pinned to `chromium-linux` baselines only — Playwright issue #13873 (closed "Not planned" 2026-05) confirms even identical official Docker images render subtly differently across CPU architectures, so cross-OS pixel equality is not a viable goal. Snapshots in `tests/e2e/specs/visual/__snapshots__/`. Update via `npx playwright test --update-snapshots --project=chromium-linux` from CI artifact replay (not from a developer's M-series Mac).
2. **Accessibility.** `@axe-core/playwright` (`new AxeBuilder({page}).withTags(['wcag2a','wcag2aa','wcag21aa']).analyze()`) on every page-level spec. Helper at `tests/e2e/helpers/axe.ts`. Fails on any non-empty `violations` array after subtracting an explicit allow-set documented in `tests/e2e/helpers/axe.ts`.
3. **User flows.** End-to-end happy paths and failure paths through the React UI: open app → grant mic → see needle move; A4 selector changes default and persists across reload (ADR-0005 + ADR-0013); device-disconnect surfaces the error UI and recovers when device returns (DESIGN.md §9.3); permissions denied shows the correct guidance.
4. **Performance.** Page-level `performance.getEntriesByType('navigation' | 'paint' | 'largest-contentful-paint' | 'layout-shift' | 'longtask')` collected in `tests/e2e/helpers/perf.ts`. FPS sampling for the tuner-needle animation via `requestAnimationFrame` loop in `tests/e2e/helpers/fps.ts`. Targets: LCP < 2.0 s, CLS < 0.05, p95 FPS > 55 on a 60 Hz display, no `longtask` > 50 ms during the steady-state tuner loop. `PerformanceEntry` objects are `JSON.parse(JSON.stringify(...))`-cloned before crossing the protocol boundary because they are not directly serialisable.
5. **Cross-browser.** Chromium + WebKit on every PR; Firefox in nightly. Microphone-dependent specs are tagged `@chromium-only` and skipped on WebKit/Firefox via `test.skip(({browserName}) => browserName !== 'chromium', 'fake mic only on chromium')` because Chromium's `--use-file-for-fake-audio-capture` flag has no Firefox or WebKit equivalent in Playwright as of 2026-06.
6. **i18n / l10n.** `test.use({locale, timezoneId})` parameterised across {en-US, de-DE, ja-JP, ar-EG} for the locales we plan to ship. Asserts: note-name formatter (ADR-0004) renders correctly in each locale; right-to-left layout in ar-EG does not break the tuner needle's left/right cents-deviation indicator; numeric formatting (frequency, cents) honours locale conventions. Note-name-system formatter is the primary unit-tested surface in Tier 1; Tier 5 confirms the wiring through to rendered DOM.

### 6.3 Mock-Tauri bridge

The mock-Tauri bridge is a single `tests/e2e/mocks/install.ts` module that Vite-bundles to a single JS file, loaded via `page.addInitScript({path: 'tests/e2e/mocks/install.bundle.js'})` from the shared fixture. The module:

1. Sets `window.__E2E__ = true` (a runtime sentinel checked by the React app to gate any production-only code paths off).
2. Calls `clearMocks()`, then `mockWindows('main')`, then `mockIPC(handler, { shouldMockEvents: true })` from `@tauri-apps/api/mocks` (2.7.0+).
3. Exposes `window.__E2E_OVERRIDE__(commandHandlers)` so individual specs can extend the IPC handler at runtime without reloading.
4. Patches both `window.__TAURI_INTERNALS__` (the 2.x global) and `window.__TAURI__` / `window.__TAURI_IPC__` (legacy, for plugins that still read them).

Production build configuration must verify the bridge is tree-shaken out — Vite's `import.meta.env.MODE === 'production'` check, combined with a dynamic `import()` of the bridge module, ensures the bundle never ships to end users. A unit assertion in `tests/e2e/specs/build-hygiene.spec.ts` greps the production bundle for `__E2E_OVERRIDE__` and fails the build if found.

## 7. Test Categories Covered (matrix: tier × category)

The five tiers cover different cross-sections of category space. This table shows where each category is exercised.

| Category                | Tier 1 (Rust unit) | Tier 2 (Philharmonia)     | Tier 3 (datasets)          | Tier 4 (benchmarks)   | Tier 5 (UI E2E)          |
| ----------------------- | ------------------ | ------------------------- | -------------------------- | --------------------- | ------------------------ |
| Algorithmic correctness | ✓ (proptest)       | ✓ (real instrument audio) | ✓ (annotated ground truth) | ✓ (publication-grade) | indirect via UI flow     |
| Visual regression       | —                  | —                         | —                          | —                     | ✓ (chromium-linux)       |
| Accessibility (WCAG)    | —                  | —                         | —                          | —                     | ✓ (every spec)           |
| User flows              | —                  | —                         | —                          | —                     | ✓                        |
| Performance / FPS       | latency p50/p99    | (via Tier-1 latency rig)  | —                          | benchmark-grade       | ✓ (browser-side)         |
| Cross-browser           | —                  | —                         | —                          | —                     | ✓ (Chromium + WebKit)    |
| i18n / l10n             | ✓ (formatter unit) | —                         | —                          | —                     | ✓ (locale parameterised) |
| Octave-error rejection  | ✓                  | ✓                         | ✓                          | ✓                     | —                        |
| Voicing / VAD           | ✓                  | ✓                         | ✓                          | —                     | ✓ (mute / no-mic flow)   |
| Tauri shell smoke       | —                  | —                         | —                          | —                     | ✓ (Track B nightly)      |

## 8. Tooling Choices

Pinned versions (exact, not range — see "Failure Policy" §11 for why):

- **`@playwright/test` 1.60.0** — published 2026-05-11. Runs the deterministic suite. `expect(page).toHaveScreenshot` is the visual-regression workhorse.
- **`@playwright/mcp` 0.0.75** — published 2026-05-07, Apache-2.0, ~33.4k stars. Pinned exact: the package has cycled through ~12 releases in the first half of 2026 (v0.0.64 → v0.0.75 between Feb and May), and `@latest` in CI is unsafe. Used out-of-band (not in the deterministic gate) for Claude-driven exploratory flows; configured in `tests/mcp/.mcp/config.json` with `--caps testing,storage --browser chromium --headless --isolated`.
- **`@axe-core/playwright`** — pinned to a known axe-core major.minor (the package tracks axe-core's major.minor, not SemVer; minor bumps can introduce new rules). Pin both `axe-core` and `@axe-core/playwright`.
- **`@tauri-apps/api` 2.11.0** — already a dep; the `/mocks` subpath exposes `mockIPC`, `mockWindows`, `clearMocks`, `mockConvertFileSrc`. `shouldMockEvents` (2.7.0+) is required for our event-driven tuner state model.
- **`tauri-driver` 2.0.6** — installed via `cargo install tauri-driver --locked --version 2.0.6` in the nightly job. License Apache-2.0 OR MIT, MSRV 1.77.2. Versioned independently from `tauri` itself (Tauri docs explicitly warn this) — do not couple bumps.
- **`@wdio/cli` 9.19.x** — pinned exact-minor for the nightly tauri-driver job. WebdriverIO 9.x auto-injects `webSocketUrl: true` to negotiate WebDriver BiDi, which is rejected by Ubuntu 22.04/24.04's pre-2.46 `webkit2gtk-driver`. Tauri issue #15415 (open as of 2026-06-03) documents the workaround: every capability block must include `'wdio:enforceWebDriverClassic': true`. This is NOT yet in the official Tauri docs and is mandatory.
- **`msedgedriver-tool`** (for Windows nightly) — installed via `cargo install --git https://github.com/chippers/msedgedriver-tool` in the nightly job to keep msedgedriver in sync with auto-updating Edge. Without it, Edge auto-updates silently break the driver and the test suite hangs on session start.
- **Linux apt packages** for the nightly tauri-driver job: `libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev librsvg2-dev libsoup-3.0-dev pkg-config webkit2gtk-driver xvfb` (the existing ci.yml already installs the first five for clippy/test/build; nightly adds `webkit2gtk-driver` and `xvfb`).
- **`@vitest/...` (existing dev-dep direction, not yet adopted)** — out of scope for this plan. Frontend unit tests are deferred; Tier 5 covers the JS surface.

We deliberately do NOT adopt:

- **Chromatic / Percy / Applitools** — for visual regression, Playwright's built-in `toHaveScreenshot` is sufficient at our scale (~5 canonical states). Chromatic's OSS tier requires 100+ contributors / 40k weekly downloads / 10k stars (we qualify on none) and the paid tier starts at $179/mo. Percy's free tier is the same 5k/month as Argos with weaker Playwright DX. Applitools has no perpetual free tier and won't quote prices publicly. **Argos-CI Hobby tier** ($0 forever, 5,000 screenshots/month, first-class Playwright SDK) is the only candidate worth re-evaluating later if reviewing PNG diffs in GitHub's diff viewer becomes painful, but is not adopted today.
- **`jest-image-snapshot` / `looks-same` / `pixelmatch` standalone** — duplicate `toHaveScreenshot`'s comparison engine (Playwright already uses `pixelmatch` internally); add a parallel test runner with no review UI; gain nothing.
- **Storybook** — not adopted just for visual testing. Reconsider only if used for design review separately.
- **Chromatic + Storybook combo** — same reasons as above; OSS tier additionally requires public Storybooks, leaking unreleased UI states.

## 9. Repo Layout (`tests/e2e/`)

```
tests/
└── e2e/
    ├── playwright.config.ts             # one config, three projects (chromium, webkit, firefox-nightly)
    ├── tsconfig.json                    # extends repo tsconfig; relaxes "noEmit"
    ├── specs/
    │   ├── tuner.spec.ts                # core flow: mic → needle → A4 selector
    │   ├── a4-selector.spec.ts          # ADR-0005: configurable A4, persistence (ADR-0013)
    │   ├── permissions.spec.ts          # mic granted / denied / not-present
    │   ├── device-disconnect.spec.ts    # DESIGN.md §9.3 recovery flow
    │   ├── i18n.spec.ts                 # locale parameterisation across en/de/ja/ar
    │   ├── visual/
    │   │   ├── tuner-states.spec.ts     # 5+ canonical needle states
    │   │   └── __snapshots__/           # *-chromium-linux.png baselines (committed)
    │   └── build-hygiene.spec.ts        # asserts __E2E_OVERRIDE__ absent from prod bundle
    ├── fixtures/
    │   ├── tauri-mock.ts                # extends base test, addInitScript bridge
    │   ├── audio.ts                     # per-test wav-path fixture for fake mic
    │   ├── clock.ts                     # page.clock.install before page.goto
    │   └── page.ts                      # composed default fixture
    ├── mocks/
    │   ├── install.ts                   # source: clearMocks + mockWindows + mockIPC
    │   ├── install.bundle.js            # vite-bundled output, committed; reproducible
    │   ├── commands.ts                  # default IPC command handlers
    │   ├── events.ts                    # default event handlers
    │   └── handlers.ts                  # composition + per-spec overrides
    ├── helpers/
    │   ├── axe.ts                       # checkA11y(page, opts)
    │   ├── perf.ts                      # measurePaint(page) → {fp,fcp,lcp,cls}
    │   ├── fps.ts                       # sampleFps(page, durationMs) → {p50,p95,min}
    │   └── localStorage.ts              # explicit reset helper (clear + IDB delete)
    └── audio/
        ├── README.md                    # how to regenerate via sox
        ├── sine-440.wav                 # A4
        ├── sine-220.wav                 # A3
        ├── silence.wav
        └── vibrato-440-5hz-20cent.wav

tests/
└── mcp/
    ├── .mcp/
    │   └── config.json                  # @playwright/mcp server config
    └── scripts/
        └── exploratory-tuner.md         # natural-language scenarios for agent runs
```

Snapshots live under `tests/e2e/specs/visual/__snapshots__/` (Playwright default for that spec subdirectory), are committed, and are checked into git. Audio fixtures are committed (small, deterministic, regeneratable via `sox`); the `audio/README.md` documents the exact `sox` invocations.

## 10. CI Workflow Additions

Two new GitHub Actions jobs in `.github/workflows/`. The existing `ci.yml` gets one new job; a new `e2e-nightly.yml` carries the tauri-driver matrix.

### 10.1 `ci.yml` — new job `e2e-mock` (every PR)

```yaml
e2e-mock:
  name: e2e-mock (${{ matrix.project }})
  runs-on: ubuntu-latest
  strategy:
    fail-fast: false
    matrix:
      project: [chromium, webkit]
  steps:
    - uses: actions/checkout@v4
    - uses: actions/setup-node@v4
      with: { node-version: "20", cache: "npm" }
    - run: npm ci
    - name: Cache Playwright browsers
      uses: actions/cache@v4
      with:
        path: ~/.cache/ms-playwright
        key: pw-${{ hashFiles('package-lock.json') }}
    - run: npx playwright install --with-deps ${{ matrix.project }}
    - run: npm run build # produces dist/ for vite preview
    - run: npx playwright test --project=${{ matrix.project }}
      env:
        VITE_E2E: "true"
    - uses: actions/upload-artifact@v4
      if: failure()
      with:
        name: playwright-report-${{ matrix.project }}
        path: |
          playwright-report/
          test-results/
```

Branch protection adds `e2e-mock (chromium)` and `e2e-mock (webkit)` to the required checks.

### 10.2 New workflow `e2e-nightly.yml` — `e2e-tauri-driver-smoke` (nightly cron)

```yaml
name: E2E nightly (tauri-driver smoke)

on:
  schedule:
    - cron: "0 7 * * *" # 07:00 UTC daily
  workflow_dispatch:

jobs:
  smoke:
    name: tauri-driver smoke (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    strategy:
      fail-fast: false
      matrix:
        os: [ubuntu-latest, windows-latest]
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
      - uses: actions/setup-node@v4
        with: { node-version: "20", cache: "npm" }
      - if: runner.os == 'Linux'
        run: |
          sudo apt-get update
          sudo apt-get install -y \
            libwebkit2gtk-4.1-dev libgtk-3-dev libayatana-appindicator3-dev \
            librsvg2-dev libsoup-3.0-dev pkg-config webkit2gtk-driver xvfb
      - run: cargo install tauri-driver --locked --version 2.0.6
      - if: runner.os == 'Windows'
        run: cargo install --git https://github.com/chippers/msedgedriver-tool
      - run: npm ci
      - run: npm run build
      - run: cargo build --release --workspace
      - name: Run wdio smoke (Linux, xvfb)
        if: runner.os == 'Linux'
        run: xvfb-run -a npm run e2e:tauri-driver
      - name: Run wdio smoke (Windows)
        if: runner.os == 'Windows'
        run: npm run e2e:tauri-driver
```

Nightly failures open an issue via `peter-evans/create-issue-from-file` (or post to the actions summary if we choose not to noisy-issue). Nightly failures **do not block** development; they surface to the maintainer for triage.

## 11. Failure Policy

### 11.1 What blocks merge

- All Tier 1 tests (existing).
- All Tier 2 fixture tests (Phase 1.4 onward).
- `e2e-mock (chromium)` (every PR).
- `e2e-mock (webkit)` (every PR).
- A failing visual-regression snapshot blocks merge until either (a) the diff is investigated and the bug is fixed, or (b) the baseline is intentionally updated via the CI-artifact replay procedure (§11.3).

### 11.2 What is informational

- `e2e-tauri-driver-smoke (ubuntu-latest)` and `(windows-latest)` from the nightly run. These are explicitly **non-blocking** because:
  - tauri-driver has multiple long-standing open bugs (#6541 `.click()` returning HTTP 500, #10670 e2e docs broken, #15415 BiDi/wdio 9 regression) that produce flake unrelated to our code.
  - Forcing a block would teach the team to ignore the signal.
  - Three days of consecutive failures on the **same** test, however, is treated as a real bug and triaged in the next sprint.
- Performance budget breaches that are within 10% of the budget produce a warning (printed to the actions summary) but do not fail. Beyond 10%, they fail. This avoids treating routine 1-frame jitter as a regression.

### 11.3 Visual baseline update process

Updating `*.png` baselines on a developer's local machine is **not allowed**: Playwright issue #13873 ("not planned") confirms even identical official Docker images render differently across host CPU architectures, so a developer's M-series Mac produces baselines that the Linux CI cannot reproduce. The supported workflow is:

1. Make the UI change in a PR.
2. CI fails the visual regression spec; the diff PNG is uploaded as an artifact.
3. The PR author reviews the diff via the artifact; if intentional, comments `/update-snapshots` on the PR.
4. A separate `update-snapshots` GitHub Actions workflow (manual `workflow_dispatch`, scoped to the PR's branch) runs `npx playwright test --update-snapshots --project=chromium-linux` on `ubuntu-latest`, commits the new baselines back to the PR branch, and pushes.
5. CI re-runs and passes.

This keeps baselines in lockstep with the one OS+arch they were generated on.

### 11.4 Flake handling

- Default `retries: 2` in CI (per `playwright.config.ts`); 0 retries locally so flakes surface immediately.
- A test that fails-then-passes on the same PR is **not** a green light. It is recorded in `tests/e2e/FLAKE_LOG.md` (one line per occurrence: date, spec, retry count, suspected cause). When any spec accumulates three flake-then-pass entries within 30 days, it is quarantined (`test.fixme`) and a fix is owed within the following sprint.
- Quarantined tests do not block merge but are tracked in `FLAKE_LOG.md`.
- The `forbidOnly: !!process.env.CI` config option blocks `test.only` from merging.

### 11.5 Pinning policy

Exact-version pinning (no `^`, no `~`) for all of: `@playwright/test`, `@playwright/mcp`, `@axe-core/playwright`, `axe-core`, `@wdio/cli` (when introduced), `tauri-driver` (cargo install --version 2.0.6 --locked). Rationale: Playwright MCP cycles ~12 releases/half-year; WebdriverIO 9.x's auto-BiDi-injection caused a 100% session-start failure (issue #15415) on stock Ubuntu CI runners until 2026-06-03 unmitigated. Until those upstream issues stabilise, defensive pinning is mandatory. Re-evaluate quarterly.

## 12. References

- [DESIGN.md](DESIGN.md) §10 — canonical four-tier description; this document is its addendum.
- [ADR-0016](../adr/0016-test-pyramid-tier-1-day-1.md) — original four-tier pyramid lock.
- [ADR-0019](../adr/0019-tier-5-e2e-playwright-mcp.md) — Tier 5 (UI E2E with Playwright MCP) lock.
- [ADR-0001](../adr/0001-license-and-foss-posture.md) — license posture (relevant to fixture licensing).
- [ADR-0003](../adr/0003-frontend-stack-react-vite-ts-zustand-tailwind-shadcn.md) — frontend stack the E2E tests target.
- [ADR-0004](../adr/0004-default-note-name-system-english-with-formatter-trait.md) — note-name formatter, exercised by i18n specs.
- [ADR-0005](../adr/0005-a4-reference-configurable-default-440.md) — A4 selector flow tested by `a4-selector.spec.ts`.
- [ADR-0013](../adr/0013-settings-via-tauri-plugin-store.md) — settings persistence asserted by `a4-selector.spec.ts`.
- [ADR-0018](../adr/0018-triple-layer-enforcement-convention-pre-commit-ci.md) — triple-layer enforcement; this plan's CI additions extend the third layer.
- Playwright docs — `https://playwright.dev/docs/test-snapshots`, `https://playwright.dev/docs/clock`, `https://playwright.dev/docs/api/class-pageassertions`.
- Playwright issue #13873 (visual cross-arch, "Not planned" 2026-05) — rationale for pinned baseline platform.
- Tauri 2.x WebDriver docs — `https://v2.tauri.app/develop/tests/webdriver/`.
- Tauri issue #7068 — macOS `tauri-driver` support (open since 2023; rationale for skipping macOS in nightly).
- Tauri issue #15415 — WebdriverIO 9.x BiDi regression on Ubuntu (open 2026-05-18; mandatory `wdio:enforceWebDriverClassic: true` workaround).
- Tauri issue #6541, #10670 — long-standing tauri-driver bugs informing the "informational only" classification of the nightly job.
- Microsoft Playwright issue #15404 — Microsoft declined Tauri support (closed 2022-07-06); rationale for browser-mode-with-mocks rather than Playwright-driving-Tauri.
- Reference implementations studied (all 2026, all confirmed working): `RandomlyZay-Labs/tauri-app-template`, `Rahuletto/mandy`, `block/sprout`, `owenisas/OmniTool`, `Swofty-Developments/CodeForge`, `jfolcini/agaric`, `hoveychen/claw-fleet`, `ScopeCreep-zip/Rekindle`.

### Open questions

- **Argos-CI adoption** — defer until reviewing PNG diffs in GitHub's diff viewer becomes painful. Free Hobby tier (5,000 screenshots/month) is sufficient at our 5-state scale.
- **Firefox in the per-PR matrix vs nightly** — start nightly only; promote to per-PR if a Firefox-specific regression is observed.
- **MCP-driven exploratory runs in CI** — explicitly out of scope for the deterministic gate. Whether to schedule a weekly MCP-driven Claude run that posts findings to a `.github/discussions` thread is open.
- **Performance budgets** — the values quoted (LCP < 2.0 s, CLS < 0.05, p95 FPS > 55, longtask < 50 ms) are starting points anchored to typical 60 Hz desktop targets. Tighten or relax after the first 30 days of measurement.
- **Locale set** — {en-US, de-DE, ja-JP, ar-EG} is the working list; final set follows the localisation roadmap (not yet ADR'd).
