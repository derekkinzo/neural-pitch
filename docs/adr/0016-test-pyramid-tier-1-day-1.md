# ADR-0016: Test pyramid: Tier 1 day 1; Tiers 2–4 phased

## Status

Accepted — 2026-06-02.

## Context

Audio testing has a tradeoff: "real" audio fixtures are large, license-encumbered, and slow to download in CI; synthesized signals are tiny and fast but only cover algorithmically clean cases. Real datasets (MAESTRO, MUSDB18-HQ, MIR-1K) are gigabytes and cannot live in the repository.

A graduated testing strategy is needed: synthesized signals as the day-1 fast inner loop, curated fixtures next, full datasets last.

## Decision

Four test tiers, phased:

| Tier | Source | Phase introduced | Gating |
|---|---|---|---|
| 1 | Synthesized signals (sine, vibrato, two-tone, noise, silence) + `proptest` invariants + `frequency_to_note` golden table (MIDI 0–127) | Phase 0 | Every `cargo test`; CI required |
| 2 | Philharmonia single-note voice fixtures, committed to `tests/fixtures/` (small, ~MB-sized subset) | Phase 1 | Every PR |
| 3 | Dataset slices (Philharmonia full, MIR-1K) fetched by `scripts/fetch-test-data.sh` to gitignored `tests/data/` | Phase 2 | `cargo test --features dataset`; not on every CI run |
| 4 | Full benchmarks (MAESTRO transcription, MUSDB18-HQ separation) | Release time | Manual; results recorded in CHANGELOG |

Phase 0 acceptance is "CI green with Tier-1 tests passing".

Tests are exempt from the `unwrap_used`/`expect_used` lints (test-cfg gate) so failures surface with full panic context.

The TDD discipline (P4) is human-enforced via PR review: every PR introducing or changing core behaviour must land the failing test first (or in the same commit, with the test authored before the implementation).

## Consequences

- The Phase-0 CI hot path stays fast (Tier-1 only).
- Phase-2 contributors who do not run `--features dataset` will not exercise Tier-3; this is acknowledged and accepted because Tier-3 is pre-merge optional.
- Tier-4 benchmarks are run before each release and recorded in the CHANGELOG so that regressions are visible.
- A Tier-1 hardware-sanity-check rig is documented (`tests/HARDWARE_RIG.md`) so that latency p50/p99 figures can be reproduced.

## Alternatives Considered

- **Single-tier (synthesized only)** — rejected because real-world recording artefacts (breath, room, mic colour) are not captured.
- **Single-tier (Philharmonia fixtures only)** — rejected because synthesized signals are how `proptest` invariants get exercised.
- **Run datasets in CI** — rejected because the CI cost (download bandwidth, compute time) is prohibitive for a personal project.
