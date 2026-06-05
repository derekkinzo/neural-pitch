# ADR-0020: Neural backends gated behind `feature = "neural"` (off by default); ONNX weights as runtime assets, not bundled

## Status

Accepted — 2026-06-04.

## Context

Phase 2.2 introduces two ONNX-backed pitch estimators — `PestoEstimator` (default neural backend) and `CrepeTinyEstimator` (MIT fallback) — plus a shared Viterbi decoder for posterior smoothing. PESTO is the strongest fit on accuracy (≈95–98% octave-correctness on MIR-1K with Viterbi), but it sits in a license gray zone:

- The PESTO inference repository is published under **LGPL-3.0**.
- The status of the PESTO **weights themselves** as redistributable artifacts is contested — counsel review of the redistribution posture is still in flight (also called out in ADR-0008 and §14 of [`../design/DESIGN.md`](../design/DESIGN.md)).

Bundling the PESTO ONNX file inside the Rust crate, or statically linking the inference path into the default binary, would expose every downstream consumer of the workspace to LGPL-3.0 reciprocal-distribution obligations. That is incompatible with the dual `MIT OR Apache-2.0` posture locked by ADR-0001.

CREPE-tiny is MIT-licensed and free of that contagion, but it still carries a multi-megabyte ONNX file and an `ort` runtime dependency that we do not want every user — including the Phase 1 / mobile / minimal-build audience — to pay for.

## Decision

- Phase 2.2 ships `PestoEstimator`, `CrepeTinyEstimator`, and the shared Viterbi decoder. **All three are gated behind `feature = "neural"`, which is OFF by default** in `crates/neural-pitch-core/Cargo.toml`.
- The base crate / default binary continues to ship YIN/MPM (Phase 1) plus pYIN (Phase 2.1, gated under `feature = "pyin"`). With no features enabled, the binary is a pure dual-MIT/Apache build with zero ONNX runtime dependency and zero ML weight files.
- **Real ONNX weights are NOT bundled with the source tree, the published crate, or the default release artifact.** Weights are treated strictly as **runtime assets**. The `models.toml` resolver (§8.5) downloads them on user opt-in via `reqwest` over HTTPS, with `fs2`-flock-coordinated writes into the platform user-data directory.
- Tests that need to exercise neural estimators end-to-end use tiny synthetic ONNX stubs or fixtures fetched by `scripts/fetch-test-data.sh`; they never check real PESTO/CREPE-tiny weights into the repo.
- The `Backend::OnnxPesto` / `Backend::OnnxCrepeTiny` arms of `make_estimator` (§5.4) return `EstimatorError::FeatureDisabled("neural")` cleanly when the feature is off, so a Phase-1-only consumer never trips a link error.

## Consequences

- **Power users and contributors who want best-in-class accuracy** opt in via `cargo build --features neural` (and a one-time `scripts/fetch-models.sh` for the weights). They take on the LGPL-3.0 redistribution question themselves at their build site.
- **Default users** continue to ship a pure dual-MIT/Apache binary with classical-only DSP. No license contagion, no ONNX runtime DLL/.so/.dylib in the bundle, no model files to host.
- The crate's published `Cargo.toml` advertises `neural` as an optional feature, so downstream Rust consumers (CLI, future mobile shell, test harnesses) inherit the same opt-in posture.
- Counsel review of PESTO redistribution remains a gating step **only** for any future "neural-on-by-default" release artifact. Current artifacts are unaffected.
- Adds a documentation burden: README and DESIGN must explain how to opt in. Mitigated by the README "Building with neural support" subsection added with this ADR.

## Alternatives Considered

- **Bundle PESTO weights as a default asset.** Rejected — exposes the workspace to LGPL-3.0 reciprocal-distribution obligations and gates every release on counsel sign-off. Direct violation of ADR-0001's dual MIT/Apache posture.
- **Drop PESTO entirely and ship only CREPE-tiny (MIT).** Rejected — both upstream research reports recommend PESTO as the strongest accuracy/latency point for voice; abandoning it discards the principal Phase 2.2 deliverable. CREPE-tiny remains in the codebase as the named MIT fallback (§14 risk row).
- **Ship PESTO inference code unconditionally and let runtime weight-absence skip it.** Rejected — the LGPL exposure attaches to the inference code linkage, not just to the weights, so a default-on inference path still triggers the contamination question.
- **Fetch PESTO weights at first launch with no Cargo feature gate.** Rejected — still pulls `ort` and `ndarray` into every build, defeating the "minimal default binary" goal and forcing every downstream consumer to ship the ONNX Runtime native dependency.
