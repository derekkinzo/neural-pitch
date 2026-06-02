# ADR-0006: Visual-only feedback Phase 1; modular for monitoring later

## Status

Accepted — 2026-06-02.

## Context

A live tuner can offer two kinds of feedback:

1. **Visual** — render the detected pitch on screen.
2. **Through-monitoring** — play the input signal to the user's headphones with low latency, optionally with effects.

Through-monitoring introduces material complexity: bidirectional audio I/O, latency budgeting tight enough that ~10 ms is audible as comb filtering against direct sound, feedback-loop risk if the user has speakers on, and platform-specific permissions (especially iOS audio-session categories).

Phase 1 is a vocal practice tool, not a stage performance tool. Visual feedback is sufficient for the primary use case.

## Decision

- Phase 1 ships **visual feedback only**. There is no through-monitoring path.
- The audio I/O abstraction (the `Capture` and future `Playback` traits) is structured so that a `MonitoringPipeline` can be added as an additive sibling to the analysis pipeline without restructuring the audio thread topology.
- The Phase 1 latency budget is sized for visual reaction (mic-to-screen p50 ≤ 45 ms, p99 ≤ 70 ms) — not for audio reinjection.
- Whether `MonitoringPipeline` ever ships, or remains a documented extension point, is deferred to Phase 2 user feedback.

## Consequences

- Phase 1 acceptance is achievable on consumer hardware without latency-tuning the user's audio stack.
- The audio thread topology (cpal callback → rtrb → DSP worker → FrameSink) does not need to accommodate a return path day 1.
- Any future addition of monitoring is purely additive: another cpal output stream, another rtrb ring, no changes to the analysis pipeline.

## Alternatives Considered

- **Ship monitoring in Phase 1** — rejected because of the complexity-vs-value imbalance for the primary use case, and because the latency budget required for monitoring would gate Phase 1 on hardware-specific tuning that the secondary persona cannot reasonably perform.
- **Defer the monitoring abstraction** — rejected because retrofitting two-way audio later would require restructuring the topology; reserving the seat day 1 is nearly free.
