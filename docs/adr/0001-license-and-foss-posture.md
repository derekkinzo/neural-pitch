# ADR-0001: License and FOSS posture

## Status

Accepted — 2026-06-02.

## Context

`neural-pitch` is a personal-and-learning project, developed in the open as a vehicle for both a usable singing-voice pitch app and the author's own study of real-time DSP, neural pitch detection, and Rust audio engineering. The author is the primary user; a small group of friends and known musicians is the secondary audience. There is no commercial release planned and no hosted-service variant in scope.

A licensing decision is required day 1 because every source file carries a license header, third-party dependencies are vetted against compatible terms, and contributors need an unambiguous contribution licence. The Rust ecosystem strongly converges on dual `MIT OR Apache-2.0`.

## Decision

The project is licensed dual `MIT OR Apache-2.0`. Both `LICENSE-MIT` and `LICENSE-APACHE` files live at the repository root. Source files do not carry per-file headers; the top-level `Cargo.toml` `license = "MIT OR Apache-2.0"` and the README are the canonical statement.

The FOSS posture is reinforced by:

- No telemetry, ever, except explicit user-initiated model downloads.
- No crash reporting.
- Local-only data residence.
- No accounts, sharing, leaderboards, or social features.

License-encumbered model weights (PESTO LGPL-3.0, MUSDB18-NC-trained separators) are correspondingly only a concern at the level of redistributing the FOSS app, not commercial sale.

## Consequences

- All third-party dependencies must be compatible with the dual licence; `cargo-deny` enforces this once configured (Phase 2+).
- Contributors sign off via a `Signed-off-by:` Developer Certificate of Origin trailer.
- A future hosted-service variant would require revisiting this decision; an ADR superseding this one would be authored at that time.
- The non-commercial framing dissolves the weight-license redistribution risk for personal use; counsel sign-off is still required before any public binary release that bundles or downloads LGPL-3.0 weights.

## Alternatives Considered

- **MIT-only** — simpler but loses the patent grant in Apache-2.0; rejected because the Rust ecosystem norm is the dual licence.
- **Apache-2.0 only** — rejected for the same ecosystem-norm reason; many downstream Rust projects refuse Apache-only deps.
- **GPL-3.0** — rejected because GPL would bar combining with permissively-licensed Rust crates we want to depend on, and forecloses on hypothetical future commercial use without offering compensating benefit for a personal project.
- **Proprietary / source-available** — rejected because the project is explicitly developed in the open as a learning vehicle.
