# Architecture Decision Records

This directory contains the locked architecture decisions for `neural-pitch`. Each ADR uses the standard Status / Context / Decision / Consequences / Alternatives Considered template. The canonical design document at [`../design/DESIGN.md`](../design/DESIGN.md) references these by number.

| ADR                                                                       | Title                                                                                      |
| ------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------ |
| [ADR-0001](0001-license-and-foss-posture.md)                              | License and FOSS posture                                                                   |
| [ADR-0002](0002-mobile-ready-repo-and-crate-shape-day-1.md)               | Mobile-ready repo and crate shape day 1                                                    |
| [ADR-0003](0003-frontend-stack-react-vite-ts-zustand-tailwind-shadcn.md)  | Frontend stack: React 19 + Vite + TS strict + Zustand + Tailwind + shadcn/ui               |
| [ADR-0004](0004-default-note-name-system-english-with-formatter-trait.md) | Default note-name system: English; multi-system formatter trait day 1                      |
| [ADR-0005](0005-a4-reference-configurable-default-440.md)                 | A4 reference: configurable day 1, default 440 Hz                                           |
| [ADR-0006](0006-visual-only-feedback-phase-1.md)                          | Visual-only feedback Phase 1; modular for monitoring later                                 |
| [ADR-0007](0007-pitch-estimator-trait-and-auto-prior.md)                  | PitchEstimator trait + auto-prior; manual instrument selector demoted to advanced settings |
| [ADR-0008](0008-phase-1-yin-mpm-only-neural-phase-2.md)                   | Phase 1 ships YIN/MPM only (pure-Rust); neural backends Phase 2                            |
| [ADR-0009](0009-phase-ordering-ear-training-before-stem-separation.md)    | Phase ordering: ear-training before stem separation                                        |
| [ADR-0010](0010-audio-formats-wav-flac-mp3-day-1.md)                      | Audio formats: WAV+FLAC+MP3 day 1; Cargo-feature-gated additions                           |
| [ADR-0011](0011-recording-defaults-48k-24bit-mono-flac.md)                | Recording defaults: 48 kHz / 24-bit / mono / FLAC                                          |
| [ADR-0012](0012-recordings-library-and-analysis-cache-sqlite.md)          | Recordings library + per-recording analysis cache in SQLite                                |
| [ADR-0013](0013-settings-via-tauri-plugin-store.md)                       | Settings via tauri-plugin-store, separate from recordings DB                               |
| [ADR-0014](0014-concurrency-tokio-stdthread-rtrb.md)                      | Concurrency: tokio for Tauri/HTTP; std::thread for DSP worker; rtrb for audio boundary     |
| [ADR-0015](0015-error-handling-thiserror-anyhow-no-panics.md)             | Error handling: thiserror in libs, anyhow in app, no panics in audio path                  |
| [ADR-0016](0016-test-pyramid-tier-1-day-1.md)                             | Test pyramid: Tier 1 day 1; Tiers 2–4 phased                                               |
| [ADR-0017](0017-observability-tracing-tauri-plugin-log-no-telemetry.md)   | Observability: tracing + tauri-plugin-log; per-frame analysis cache; no telemetry          |
| [ADR-0018](0018-triple-layer-enforcement-convention-pre-commit-ci.md)     | Triple-layer enforcement: convention + pre-commit + CI                                     |
| [ADR-0019](0019-tier-5-e2e-playwright-mcp.md)                             | Tier 5 UI E2E: Playwright MCP browser-mode per-PR + tauri-driver nightly Linux/Windows     |
