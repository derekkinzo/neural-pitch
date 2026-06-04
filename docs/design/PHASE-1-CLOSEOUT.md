# Phase-1 Closeout Summary

## Status

**Closed.** Phase 1 closed on `2026-06-03` at commit `25b4b9c5b3612cb806316fe3d53b339c280343dd`. This document is the matched closeout for the acceptance contract in [`DESIGN.md`](./DESIGN.md) §13.2: every Phase-1 deliverable from §13.2 has landed, the Tier-1 / Tier-2 harness is green on the Tier-1 hardware rig, and the acceptance harness emits a JSON report whose latest values are linked from the [Acceptance results](#acceptance-results) section below.

Numbers in this document are rendered at release time from [`../reports/phase-1-acceptance.json`](../reports/phase-1-acceptance.json) — the closeout text never inlines a hard-coded score. The commit-SHA placeholder above is rewritten in-place by `scripts/run-acceptance.sh` after the harness produces a real report.

## Phase-1 deliverables shipped

One line per sub-phase. Citations point at the design sections that own the detail; this closeout deliberately does not restate design content. Tier 1 (synthesised + proptest + golden table) and Tier 2 (synthetic voice fixtures) are both wired and gating every PR — per-tier counts (`tier_1_count`, `tier_2_count`) live in [`../reports/phase-1-acceptance.json`](../reports/phase-1-acceptance.json) alongside `aggregate`, `latency_p50_ms`, and `latency_p99_ms`.

- **1.0** — `YinMpmEstimator` impl in `crates/neural-pitch-core` ([`DESIGN.md`](./DESIGN.md) §5, §13.2).
- **1.1** — `cpal` capture + `rtrb` hand-off + DSP worker in `src-tauri/` ([`DESIGN.md`](./DESIGN.md) §6, §9.2).
- **1.2** — `tauri::ipc::Channel<PitchUpdate>` stream via the `FrameSink` trait + React tuner page + A4 selector ([`DESIGN.md`](./DESIGN.md) §7).
- **1.3** — Auto-prior (running F0 median + power-weighted F0 histogram) + platform entitlements ([`DESIGN.md`](./DESIGN.md) §5; ADR-0007).
- **1.4** — Synthetic-voice fixture harness ([`DESIGN.md`](./DESIGN.md) §10.1; ADR-0016) + this closeout ([`DESIGN.md`](./DESIGN.md) §13.2).

## Acceptance results

The Phase-1 acceptance criterion is octave-correctness ≥ 95% on the Philharmonia voice subset with no manual instrument selection ([`DESIGN.md`](./DESIGN.md) §13.2). The aggregate score, per-fixture pass/fail, and mic-to-screen latency p50 / p99 figures are sourced from [`../reports/phase-1-acceptance.json`](../reports/phase-1-acceptance.json); the harness records latency p50 and p99 alongside octave-correctness in the same report so closeout numbers move together.

## Deferred items (not fully covered by 1.5)

Items the test-plan pass touched but did not close:

- **Tier-3 dataset wiring** — Bach10, MDB-stem-synth subset, GuitarSet. Scaffolding only; not gating CI.
- **Tier-5 E2E coverage of the recordings flow** — Phase 2 dependency, intentionally stubbed.
- **PESTO / CREPE neural backends** — locked to Phase 2 by ADR-0008.
- **`debug-overlay` Cargo feature** — declared in [`DESIGN.md`](./DESIGN.md) §5.5 but emits no production frames yet.

## Open risks heading into Phase 2

Four explicit risks, each with a one-line mitigation pointer:

- **ML inference surface.** PESTO LGPL counsel review ([`DESIGN.md`](./DESIGN.md) §14; ADR-0008); fallback is CREPE-tiny (MIT) with no crate API change.
- **Recording path.** First write-side I/O consumer beyond `src-tauri/` — first I/O consumer beyond the visual-only Phase-1 floor (see ADR-0006); may force the `crates/neural-pitch-io` split deferred in [`DESIGN.md`](./DESIGN.md) §15. Persistence boundary specified by ADR-0012.
- **Vocal range detection.** New analyser surface; must respect P3 (audio callback is sacred) and P8 (additive, never restructure the live tuner).
- **Model resolver / network.** First non-test network egress; janitor + flock semantics from [`DESIGN.md`](./DESIGN.md) §8.5 land for real.

## Open questions

Listed, not answered, in this closeout. Resolution lands at the Phase 2 entry pass.

- Should AutoPrior remain `Generic`-default in Phase 2 once an explicit-instrument picker exists in the UI? Auto-by-default was locked in ADR-0007, but a UI selector changes the friction trade-off.
- Should Tier-3 datasets (Bach10, MDB-stem-synth, GuitarSet) graduate to gating CI at the Phase 2 boundary, or do the synthetic + Philharmonia fixtures stay sufficient indefinitely?
- Should MUSDB18 vocal-stem fixtures be added at Phase 2? The non-commercial license is fine for personal / internal use but blocks any future commercial fork — needs to be a conscious call, not drift.

## Pointers

- [`DESIGN.md`](./DESIGN.md) §13.2 — Phase 1 acceptance contract.
- [`DESIGN.md`](./DESIGN.md) §10.1 — test-pyramid tier table.
- [`DESIGN.md`](./DESIGN.md) §14 — cross-cutting risks.
- [`DESIGN.md`](./DESIGN.md) §15 — open questions and deferred decisions.
- [`RESEARCH-REPORT.md`](../research/RESEARCH-REPORT.md) §12 — tiered-test methodology evidence.
- ADR-0006 — visual-only feedback floor for Phase 1 (no through-monitoring).
- ADR-0007 — `PitchEstimator` trait / auto-prior default.
- ADR-0008 — neural backend phasing (PESTO Phase 2; CREPE-tiny fallback).
- ADR-0012 — recordings library and analysis-cache persistence boundary.
- ADR-0016 — test pyramid: Tier 1 day 1; Tiers 2–4 phased.
- [`../reports/phase-1-acceptance.json`](../reports/phase-1-acceptance.json) — generated acceptance report.
