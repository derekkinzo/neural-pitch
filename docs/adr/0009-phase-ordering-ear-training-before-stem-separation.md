# ADR-0009: Phase ordering: ear-training before stem separation

## Status

Accepted — 2026-06-02.

## Context

The original phase ordering had stem separation (HTDemucs / BS-RoFormer) at Phase 4 and ear-training games at Phase 5. Stem separation is the headline ML feature; ear-training is the headline learning feature.

Two pressures motivated revisiting the order:

1. **Build on tested foundations.** Ear-training drills exercise the live tuner and recording paths — the same paths users hit every day. Bugs surface against features actively in use, not features under construction.
2. **Stem separation is heavyweight.** HTDemucs requires ONNX export, GPU acceleration tuning, MUSDB18-HQ benchmarking, and licensing review for non-commercial training data. Tackling it after the simpler features have shaken out reduces the risk of a long Phase-4 stall blocking everything downstream.

## Decision

The roadmap is reordered:

- **Phase 4** = ear-training games (movable-do solfege drills, Smule-style karaoke pitch ribbon).
- **Phase 5** = stem separation (HTDemucs default; BS-RoFormer additive when GPU is detected; per-stem detector dispatch).

## Consequences

- The SoundFont synthesiser (Phase 4 dependency) and the second `NoteFormatter` impl (movable-do) land before the heavyweight ML pipeline.
- Phase 4 acceptance is measurable end-to-end on a single laptop without GPU.
- Stem separation can take whatever time it needs in Phase 5 without blocking the ear-training feature set.
- The stem-separator dispatch model (per-stem `PitchEstimator`) is unchanged; only the calendar moves.

## Alternatives Considered

- **Keep the original ordering** — rejected because it puts the riskiest, most license-encumbered feature ahead of the lower-risk, higher-user-value ear-training feature.
- **Run them in parallel** — rejected because the project has one developer and parallel phases mean neither finishes.
- **Skip stem separation entirely** — rejected because per-track notes is one of the two stated use-case priorities (`RESEARCH-REPORT.md` §1).
