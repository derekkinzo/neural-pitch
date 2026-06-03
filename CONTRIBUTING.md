# Agent rulebook

Operational rules for anyone (human or AI agent) committing to this repo.

## Local validation gate

Before any push:

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
npm run typecheck
npm run lint
npm run format -- --check
```

`scripts/check-all.sh` runs the same gate. All six must be clean. CI runs the same gate plus a Linux/macOS/Windows × stable/beta test matrix and a `cargo deny` audit.

## Commit format

Kernel-style: `<subsys>: <imperative subject>` (≤72 chars, no trailing period). Body explains _why_, wrapped at 72 cols. Final paragraph carries a DCO `Signed-off-by:` trailer (`git commit -s`).

Allowed subsystems: `core`, `ui`, `tauri`, `audio`, `dsp`, `ml`, `ci`, `docs`, `build`, `test`, `chore`.

CI rejects commits that fail subject-format or DCO checks.

## Code rules (enforced by lints)

- No `unsafe` (`unsafe_code = "forbid"`).
- No `unwrap` / `expect` / `panic!` in production code. Tests are exempt with a top-of-file `#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]`.
- `clippy::pedantic` warns; warnings are errors.
- Inclusive language — denylist/allowlist (not black/whitelist), primary/replica (not master/slave).

## Architecture

[docs/design/DESIGN.md](docs/design/DESIGN.md) is authoritative.
