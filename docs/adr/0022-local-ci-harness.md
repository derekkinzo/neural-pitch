# ADR-0022: Local CI Harness (Three-Tier Pre-Push Gate)

## Status

Accepted — 2026-06-06.

## Context

Four of the last eight pushes shipped with red CI. Every failure was
preventable: feature-gate omission (`--no-default-features` build
broke), prettier drift, Docker font drift in visual tests, and a runner
quota outage that masked a real clippy warning. The existing local gate
(`scripts/check-all.sh`) ran only 6 of CI's 14 checks — the gap was the
failure surface.

The hard rule is non-negotiable: **CI is always green, zero warnings,
zero errors, both `--all-features` and `--no-default-features` build
clean and pass tests**. Test files using feature-gated symbols MUST
start with `#![allow(missing_docs)]` then `#![cfg(feature = "...")]`.
Channel-based test patterns MUST tolerate the receiver closing early
(Windows scheduling). Visual baselines are regenerated inside the
official Playwright Docker image so they never drift due to host font
or renderer differences.

## Decision

Replace `scripts/check-all.sh` with `scripts/ci-local.sh`, a three-tier
harness:

- **`quick`** (~3 min warm cache): fmt, clippy `-D warnings`, build +
  test under `--all-features` and `--no-default-features`, `cargo
+beta test` (when beta toolchain is present), `cargo deny`, release
  build, voice-acceptance harness, tsc (app + e2e), eslint, prettier,
  no-leak grep. Wired as the `pre-push` hook in
  `.pre-commit-config.yaml`.
- **`visual`** (~90 s): Playwright visual tests inside the official
  Docker image. Run when UI changes.
- **`full`** (~10 min): full `ci.yml` emulation via `act`. Run when CI
  config or workflow files change.

Pre-push installation is added to `scripts/install-hooks.sh`:

```sh
pre-commit install --hook-type pre-commit
pre-commit install --hook-type commit-msg
pre-commit install --hook-type pre-push
```

`git push --no-verify` is the only documented bypass — "for genuine
emergencies only — CI will catch you anyway."

## Consequences

- Every push pays ~30 s for `quick`. UI changes pay ~90 s for `visual`.
  CI-affecting changes pay ~10 min for `full`.
- Red CI on `main` becomes a near-impossibility — the gate catches
  every class of failure observed in the last 8 pushes (missing
  feature-gate, prettier drift, font-drift in visual baselines, masked
  clippy warnings).
- The `quick` tier is small enough (~30 s) that developers will not
  reflexively `--no-verify`. The `visual` and `full` tiers are
  opt-in escalations, not always-on costs.
- `scripts/check-all.sh` is removed; its callers (CI, developer
  muscle-memory) move to `scripts/ci-local.sh quick` or higher.
- The Playwright Docker image is the single source of truth for
  visual-baseline rendering; baselines authored on a developer's
  laptop are forbidden (matches TEST-PLAN.md §11.3).

## Alternatives Considered

- **Rely on CI alone.** Rejected: feedback loop is 10+ minutes,
  runner outages hide failures, and developers context-switch before
  results land. Four of the last eight pushes shipped red precisely
  because of this pattern.
- **sccache for faster CI.** Orthogonal — speeds up CI but does not
  catch missing-feature-gate bugs or prettier drift before push.
- **Post-commit hook.** Rejected: too late if the commit is bad;
  `pre-push` is the last reversible gate before history hits the
  remote.
- **Two-tier (quick + full only).** Rejected: visual-baseline
  regeneration genuinely requires the Docker image; folding it into
  `full` would force a 10-minute wait for what is properly a 90-second
  check, and developers would skip it.

## References

- ADR-0018 — triple-layer enforcement (convention + pre-commit + CI).
- ADR-0019 — Tier-5 Playwright E2E (visual-baseline determinism).
- `docs/design/TEST-PLAN.md` §"Pre-push gate" — concrete tier mechanics.
- `docs/design/DESIGN.md` §12 — Conventions and Enforcement.
- `CONTRIBUTING.md` — three-tier table for contributors.
