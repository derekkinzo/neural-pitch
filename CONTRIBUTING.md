# Agent rulebook

Operational rules for anyone (human or AI agent) committing to this repo.

## Local validation gate (three-tier)

CI must always be green: zero warnings, zero errors, both
`--all-features` AND `--no-default-features` build clean and pass tests.
The local harness at `scripts/ci-local.sh` is the canonical pre-push gate
and exists so red CI is impossible. ADR-0022 records the rationale.

| Tier     | Command                      | Wall-clock | Catches                                                                                                                                                                                                                           |
| -------- | ---------------------------- | ---------: | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `quick`  | `scripts/ci-local.sh quick`  |     ~3 min | fmt, clippy `-D warnings`, `--all-features` + `--no-default-features` build/test, `cargo +beta test`, `cargo deny`, release build, voice-acceptance harness, tsc app + e2e, eslint, prettier, no-leak grep (internal-ref / paths) |
| `visual` | `scripts/ci-local.sh visual` |      ~90 s | Playwright visual baselines (Chromium + WebKit) inside the official Docker image (font/render determinism)                                                                                                                        |
| `full`   | `scripts/ci-local.sh full`   |     ~10 m. | Full `ci.yml` emulation via `act` — every Linux-runnable job. Test matrix (macOS / Windows legs) defers to remote CI                                                                                                              |

The exact step list and CI-job coverage map lives at the top of
`scripts/ci-local.sh` and is the single source of truth — the table
above is a summary. Cold-cache release rebuilds in `quick` can push
the `quick` tier to ~5-10 minutes; warm cache after a `cargo build
--release` is closer to the ~3-minute target.

`quick` runs automatically as the `pre-push` hook (see
`.pre-commit-config.yaml`). Escalate to `visual` when your change
touches the React UI; escalate to `full` when your change touches
`.github/workflows/`, the workspace `Cargo.toml`, or `package.json`.

**Bypass policy.** `git push --no-verify` is the **only** escape hatch.
For genuine emergencies only — CI will catch you anyway. Any push that
bypasses the gate must be followed by a green CI run before the next
merge.

### First-time setup checklist

- [ ] Rust toolchain (`rust-toolchain.toml` pins the channel)
- [ ] Node 20+ and `npm ci`
- [ ] `pre-commit` (`pipx install pre-commit`)
- [ ] Docker (running daemon, used for `visual` + `full` tiers)
- [ ] `act` (CI emulator; install below)
- [ ] `scripts/install-hooks.sh` (installs both `pre-commit` and `pre-push` stages)

Install `act`:

```sh
# Linux — the official install.sh defaults BINDIR to ./bin (cwd-relative),
# so we pin BINDIR with `-b /usr/local/bin` to land it on PATH.
curl --proto '=https' --tlsv1.2 -sSf \
  https://raw.githubusercontent.com/nektos/act/master/install.sh \
  | sudo bash -s -- -b /usr/local/bin

# macOS
brew install act

# verify
act --version    # expect >= 0.2.60
```

### Troubleshooting

- `docker: pull access denied` — run `docker login`; the Playwright
  image is public but rate-limited for anonymous pulls.
- `act: workflow file not found` — run from repo root; `act` resolves
  `.github/workflows/ci.yml` relative to cwd.
- Visual diff failures after a font/OS update — regenerate baselines
  with `scripts/update-visual-baselines.sh` (which runs inside the same
  Docker image CI uses, so baselines never drift due to host font/
  renderer differences).

## Commit format

Kernel-style: `<subsys>: <imperative subject>` (≤72 chars, no trailing period). Body explains _why_, wrapped at 72 cols. Final paragraph carries a DCO `Signed-off-by:` trailer (`git commit -s`).

Allowed subsystems: `core`, `ui`, `tauri`, `audio`, `dsp`, `ml`, `ci`, `docs`, `build`, `test`, `chore`.

CI rejects commits that fail subject-format or DCO checks.

## Code rules (enforced by lints)

- No `unsafe` (`unsafe_code = "forbid"`).
- No `unwrap` / `expect` / `panic!` in production code. Tests are exempt with a top-of-file `#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]`.
- `clippy::pedantic` warns; warnings are errors.
- Inclusive language — denylist/allowlist (not black/whitelist), primary/replica (not master/slave).
- Test files using feature-gated symbols MUST start with `#![allow(missing_docs)]` then `#![cfg(feature = "...")]`.
- Channel-based test patterns MUST tolerate the receiver closing early
  (Windows scheduling can drop the receive side before the sender's
  final push).

## Architecture

[docs/design/DESIGN.md](docs/design/DESIGN.md) is authoritative.
[docs/design/TEST-PLAN.md](docs/design/TEST-PLAN.md) §"Pre-push gate"
documents the three-tier harness.
