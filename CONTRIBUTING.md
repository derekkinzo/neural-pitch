# Contributing to NeuralPitch

Thanks for your interest in contributing. This document describes how to set
up your environment, format commits, and pass CI on the first try.

## Development setup

1. Clone the repository.
   ```sh
   git clone https://github.com/derekkinzo/neural-pitch.git
   cd neural-pitch
   ```
2. Install the Rust toolchain pinned by `rust-toolchain.toml`. With `rustup`
   installed, simply running any cargo command in the workspace will fetch
   the right channel.
3. Install Node.js **20 or newer**.
4. Install the JavaScript dependencies with `pnpm` (preferred) or `npm`:
   ```sh
   pnpm install   # or: npm install
   ```
5. Install the Git hooks. **This step is mandatory** — bypassing local hooks
   will produce CI failures on the same checks:
   ```sh
   scripts/install-hooks.sh
   ```

## Commit message format

NeuralPitch uses kernel-style commit messages. Each commit subject must
follow:

```
<subsys>: <imperative subject>
```

- The subject line is at most 72 characters, written in the imperative mood
  ("add", "fix", "refactor" — not "added"/"fixes"/"refactored"), and has no
  trailing period.
- The body explains **why** the change is being made — not what changed (the
  diff already tells you that). Wrap the body at 72 columns.
- The body is separated from the subject by a single blank line.
- Trailers go in the final paragraph. Every commit ends with a
  `Signed-off-by:` trailer (DCO — see below).

Allowed `<subsys>` prefixes:

| Prefix | Scope                                                  |
| ------ | ------------------------------------------------------ |
| core   | `crates/neural-pitch-core` and other Rust core crates  |
| ui     | Web frontend (React / Vite / TypeScript)               |
| tauri  | `src-tauri/` shell, IPC, and packaging                 |
| audio  | Capture, devices, ring buffers                         |
| dsp    | Pitch detection, filters, signal-processing primitives |
| ml     | Future ML / model integration                          |
| ci     | GitHub Actions, workflow configuration                 |
| docs   | Documentation, including ADRs and research reports     |
| build  | Build scripts, Cargo / package manifests, toolchain    |
| test   | Test-only changes                                      |
| chore  | Repo housekeeping that does not fit the above          |

Example:

```
dsp: switch default detector to YIN

The autocorrelation prototype has frequent octave errors in the
soprano range, which directly hurts the primary "find your range"
use case. YIN is the well-studied baseline for monophonic pitch
detection and is what most reference implementations compare against,
so adopting it as the default lowers the surprise factor for
contributors familiar with the literature.

Signed-off-by: Jane Doe <jane@example.com>
```

## DCO sign-off

Every commit must carry a Developer Certificate of Origin
([https://developercertificate.org/](https://developercertificate.org/))
sign-off. Configure the alias once and use `git cs` everywhere:

```sh
git config alias.cs "commit -s"
git cs -m "core: add ring buffer skeleton"
```

CI rejects unsigned commits.

## Trailers

- `Fixes: <12-char-sha> ("subject")` when a commit fixes a previous one. Use
  `git log --abbrev=12 --pretty=oneline` to grab the canonical short SHA and
  subject.
- `Reviewed-by: Name <email>` and `Tested-by: Name <email>` are optional and
  primarily used once the project has multiple regular reviewers.

## Branch and PR conventions

- Feature branches use the prefix `topic/`, for example
  `topic/yin-detector` or `topic/tauri-mic-permission`.
- Pull requests use **squash-merge only**. No merge commits land on `main`;
  the squashed message must follow the commit format above.

## Pre-commit hooks

`scripts/install-hooks.sh` configures hooks that run on every commit:

- `cargo fmt -- --check` (Rust formatting)
- `cargo clippy --all-targets --all-features -- -D warnings`
- `prettier --check` on the frontend
- `eslint` on the frontend
- `tsc --noEmit` for TypeScript type checking
- `commit-msg` validation (subject prefix, line length, DCO sign-off)

Bypassing hooks (`git commit --no-verify`) is **not** an escape hatch — the
same checks run in CI as required jobs and a bypass becomes a CI failure.

## CI

GitHub Actions enforces the following required jobs on every PR:

- `commit-lint` — validates message format and DCO trailer for every commit
  in the PR.
- `fmt` — `cargo fmt -- --check`.
- `clippy` — `cargo clippy --all-targets --all-features -- -D warnings`.
- `typecheck` — `tsc --noEmit` for the web frontend.
- `test` — Rust and frontend test suites across the supported OS matrix.
- `build` — `cargo build` for the workspace and a Tauri release build.

## Code style

**Rust**

- Formatting: `rustfmt` (default config; CI enforces).
- Lints: `clippy::pedantic` plus warn-as-error. New `#[allow(...)]` annotations
  must be justified in a code comment.
- No `unwrap`, `expect`, or `panic!` in production code paths. Tests are
  exempt. Prefer typed errors and `?` propagation.

**TypeScript**

- `tsconfig` runs in `strict` mode.
- Formatter: `prettier`. Linter: `eslint`. Both run in CI.

## Inclusive language

We use inclusive terminology in code, comments, commit messages, and
documentation. The following terms are not allowed:

| Don't use | Use instead                              |
| --------- | ---------------------------------------- |
| master    | primary, main, leader, controller        |
| slave     | replica, secondary, follower, responder  |
| blacklist | denylist, blocklist, exclusion list      |
| whitelist | allowlist, approved list, inclusion list |

## Architecture context

For the system-wide design, module boundaries, and the phased roadmap, see
[docs/design/DESIGN.md](docs/design/DESIGN.md).
