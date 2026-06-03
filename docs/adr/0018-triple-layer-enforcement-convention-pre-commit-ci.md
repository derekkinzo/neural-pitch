# ADR-0018: Triple-layer enforcement: convention + pre-commit + CI

## Status

Accepted — 2026-06-02.

## Context

A project-of-one with occasional contributors needs enforcement that:

- catches issues _before_ they get committed (cheap),
- catches issues _before_ they get merged (cheaper than catching them in production),
- and is documented enough that a contributor who cloned five minutes ago can comply without surprise.

Relying on convention alone fails the moment the developer is tired. Relying on CI alone wastes minutes per commit on issues that a 200ms local hook would catch. Relying on pre-commit alone fails the moment someone forks without installing hooks.

## Decision

Three layers, in order of prevention strength:

1. **Convention** — `CONTRIBUTING.md` documents:
   - Linux-kernel-style commit messages: imperative subject under 72 chars; body wraps at 72; `Fixes:` and `Signed-off-by:` trailers.
   - DCO via `Signed-off-by:` (`git commit -s`).
   - Branch-naming and review flow.

2. **Pre-commit hooks** (installed via `scripts/install-hooks.sh`):
   - `cargo fmt --check`
   - `cargo clippy -D warnings`
   - `cargo deny check` (when `deny.toml` is configured, Phase 2+)
   - `prettier`
   - `eslint --max-warnings 0`
   - `tsc --noEmit`
   - `commit-msg` script: subject length, imperative-mood check (fails on `*ed|*ing` first words; accepts the false-positive cost), DCO trailer present.
   - Trailing-whitespace, EOF-newline, large-file rejection.

3. **CI** (GitHub Actions `ci.yml`):
   - All pre-commit checks.
   - Test matrix Linux/macOS/Windows × stable/beta.
   - `cargo deny` (Phase 2+).
   - Tauri bundle smoke build.
   - Branch protection requires `commit-lint`, `fmt`, `lint`, `typecheck`, all `test-matrix` cells, and `build`. `deny` is required only when `deny.toml` is populated; branch protection is updated at that time.

Workspace lints (`Cargo.toml [workspace.lints]`):

- `unsafe_code = "forbid"` (hard ban; cannot be relaxed by inner `#![allow]`).
- `clippy::pedantic = "warn"`.
- `clippy::unwrap_used = "deny"`.
- `clippy::expect_used = "deny"`.
- `clippy::panic = "deny"`.
- `clippy::todo = "warn"`.
- `missing_docs = "warn"`.

`.cargo/config.toml`: `rustflags = ["-D", "warnings"]`.

## Consequences

- A contributor who installs hooks gets the same fast feedback locally that CI gives.
- A contributor who skips hooks still gets caught in CI; nothing merges with a dirty tree.
- The `unsafe_code = "forbid"` ban is genuinely absolute; relaxing it requires a workspace lint change and an ADR.
- The imperative-mood check produces occasional false positives; the cost is small and the discipline payoff is consistent commit history.

## Alternatives Considered

- **CI only** — rejected because CI feedback is too slow for inner-loop iteration.
- **Pre-commit only** — rejected because contributors can fork without installing hooks.
- **`unsafe_code = "deny"` instead of `"forbid"`** — rejected because the team has no current `unsafe` need; if one arises, the workspace lint can be relaxed and an ADR records the relaxation.
- **Conventional Commits format** — rejected in favour of Linux-kernel style; the kernel style's body-first-line / `Signed-off-by:` discipline is the explicit goal.
