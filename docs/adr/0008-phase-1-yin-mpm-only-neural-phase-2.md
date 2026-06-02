# ADR-0008: Phase 1 ships YIN/MPM only (pure-Rust). Neural backends Phase 2.

## Status

Accepted — 2026-06-02.

## Context

Three pitch-detection options were on the table for Phase 1:

1. **YIN/MPM (pure-Rust, classical)** — well-understood algorithms, ~5–10 ms per frame at 2048/48 kHz on a modern laptop, no model weights to ship, no LGPL exposure, mobile-friendly CPU profile.
2. **PESTO ONNX (neural)** — state-of-the-art accuracy, ~95–98% octave-correctness on MIR-1K with Viterbi, requires `ort` 2.0-rc + ONNX Runtime native dep, LGPL-3.0 inference repo (weights as data, counsel review pending).
3. **CREPE-tiny ONNX (neural)** — MIT-licensed fallback to PESTO, slightly lower accuracy.

Phase 1 is the first user-visible release; it must be reliable, low-friction to ship, and free of redistributable-licence questions.

## Decision

- **Phase 1 ships YIN/MPM only.**
- The trait abstraction (ADR-0007) makes Phase 2 plug-in mechanical.
- YIN/MPM stays in the codebase forever as a low-CPU / mobile fallback and as a cross-check for neural backends.
- The Phase-1 auto-prior is built on a running F0 median + power-weighted F0 histogram (no ML).
- Neural backends arrive in Phase 2 behind `feature = "neural"`.
- The default neural backend in Phase 2 is PESTO (pending counsel review of LGPL redistribution); CREPE-tiny is the MIT fallback if counsel rejects.

## Consequences

- Phase 1 has zero ONNX runtime dependency, zero model weights to ship, and no licence-review gating step.
- Phase 1 acceptance (octave-correctness ≥ 95% on Philharmonia voice fixtures with no manual instrument selection) is achievable on YIN+auto-prior.
- The `feature = "neural"`-off build remains useful in Phase 2+ as a "native runtime not available" or "minimal mobile" build profile.
- YIN/MPM's slight accuracy gap versus PESTO on edge cases (sub-octave subharmonics, breath, fast onsets) is accepted for Phase 1.

## Alternatives Considered

- **Ship PESTO in Phase 1** — rejected because it gates the first release on counsel sign-off and on `ort` 2.0-rc stability.
- **Ship pYIN in Phase 1** — rejected because pYIN's HMM smoothing adds latency and the marginal accuracy lift over YIN+auto-prior is small.
- **Ship a CLI-only Phase 1 with no DSP** — rejected because the Tauri scaffolding is the harder integration; deferring DSP makes the first release feel inert.
