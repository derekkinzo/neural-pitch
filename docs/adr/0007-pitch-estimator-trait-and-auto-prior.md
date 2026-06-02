# ADR-0007: PitchEstimator trait + auto-prior; manual instrument selector demoted to advanced settings

## Status

Accepted — 2026-06-02.

## Context

The pitch-detection landscape includes classical signal-processing methods (YIN, MPM, pYIN) and learned models (CREPE, CREPE-tiny, PESTO). Each has its own latency, CPU cost, accuracy profile, license, and runtime requirements. A monolithic implementation that bakes in one method would be costly to evolve.

A second design pressure: many tuner apps require the user to pick an instrument (Guitar / Bass / Voice / Piano) before pitch detection runs. The primary persona for `neural-pitch` is a singer; the dominant signal-processing improvement they want is "the app just works when I sing into it", not "the app gives me a more correct guitar reading after I tell it I'm playing guitar".

`MODULAR-PITCH-RESEARCH.md` surveys both pressures and recommends a trait abstraction plus an auto-prior pipeline.

## Decision

The pitch-detection surface is a `PitchEstimator` trait, defined in `crates/neural-pitch-core/src/pitch/mod.rs`:

```rust
pub trait PitchEstimator: Send {
    fn name(&self) -> &'static str;
    fn config(&self) -> &EstimatorConfig;
    fn process(&mut self, samples: &[f32]) -> Result<Option<F0Frame>, EstimatorError>;
    fn reset(&mut self);
}
```

Key shape decisions:

- `&mut self` on `process` (PESTO threads a cache tensor; pYIN carries HMM history).
- `Option<F0Frame>` return distinguishes "no decision" from "low-confidence decision".
- `Send` bound only — no `Sync` bound; estimators are not designed to be shared across threads.
- Object-safe: no associated types, no generic methods. Default downstream form is `Box<dyn PitchEstimator>`.

The trait shape is fixed in Phase 0. Backends are added behind Cargo features (`pyin`, `neural`).

The default UX for singing is **auto-instrument**, not a manual selector. The selector is demoted to advanced settings for explicit guitar/bass/piano modes. The auto-prior pipeline is staged:

- **Phase 1**: running F0 median + power-weighted F0 histogram bootstrap (classical, no ML).
- **Phase 2**: YAMNet-style classifier prior added behind `feature = "neural"`.

## Consequences

- Adding a new monophonic backend is a new file, a `Backend` enum variant, and a `match` arm in `make_estimator`. No pipeline code changes.
- The trait-level pipeline code (`LiveTunerPipeline`, `SongAnalysisPipeline`) is backend-agnostic.
- Manual instrument selection still exists for users who want it, but is hidden behind advanced settings; the singer persona never sees it.
- Calibration of `confidence` differs across backends; cross-backend confidence thresholds are not meaningful.

## Alternatives Considered

- **Free function pipeline with a `Backend` enum** — rejected because backends genuinely need per-instance state (PESTO cache, pYIN history); a trait is a more honest abstraction.
- **Generic over the backend** — rejected because every pipeline would need to be generic, defeating the point of swappable backends.
- **Manual instrument selector as the default** — rejected per `MODULAR-PITCH-RESEARCH.md` recommendation and the singing-voice primary use case.
- **Skip the auto-prior** — rejected because YIN/MPM without a prior octave-halves on low voices and octave-doubles on bright tones; the auto-prior is the dominant accuracy lift in Phase 1.
