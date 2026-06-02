# Repository Conventions Research Report — NeuralPitch

**Status:** Architecture Decision Document
**Date:** 2026-06-02
**Scope:** Tauri 2.x desktop app + Rust core crate (multi-platform target including iOS/Android), GitHub-hosted OSS project.

---

## Executive Summary

For a Tauri 2.x project with a shared Rust core targeting desktop and mobile, the verified-evidence answer is: use **lowercase kebab-case** (`neural-pitch`) for the GitHub repository, Cargo package name, and all workspace-member crates, while keeping **"NeuralPitch"** as the display brand (README title, Tauri bundle product name). The **Cargo 2024 edition** is the current `cargo new` default and is the right baseline. Adopt the standard Rust **dual-license `MIT OR Apache-2.0`** as an SPDX expression in `Cargo.toml`. For repo layout, follow the Tauri 2.x canonical shape (web frontend at root, Rust in `src-tauri/`) but enroll `src-tauri/` as a member of a top-level Cargo workspace alongside split crates (`crates/` directory) for `core`, `io`, and `ml` — a pattern directly demonstrated by Tauri's own `examples/api`. Several stylistic conventions (commit policy, sign-off, MSRV, clippy levels, license headers, CHANGELOG) are addressed in Sections 4–6 with the caveat that some rest on weaker evidence than the structural claims and are flagged accordingly.

---

## 1. Naming

### 1.1 What Cargo and rustc actually require

| Concept | Rule | Source |
|---|---|---|
| Cargo **package** name | Alphanumerics + `-` or `_`; non-empty. Hyphens permitted. | Cargo Manifest reference. |
| Rust **crate** name (used by rustc, `extern crate`, `use`) | Must be a valid Rust identifier. **Hyphens disallowed.** | Rust Reference; RFC 940. |
| Default mapping | When `Cargo.toml` does not set an explicit `[lib]`/`[[bin]]` `name`, Cargo passes `--crate-name <pkg-with-_>` to rustc, replacing every `-` with `_`. | RFC 940; Cargo `cargo-targets` reference. |
| Casing convention | Modules `snake_case`; types `UpperCamelCase`. **Crate-level casing is explicitly marked `unclear`** in the Rust API Guidelines naming table. | rust-lang.github.io/api-guidelines/naming.html. |

**Implication.** A package called `neural-pitch` is referenced in code as `use neural_pitch::…` automatically. The package name and crate name are *two distinct concepts* (RFC 940 makes this explicit). You may name the package `neural-pitch` with no friction whatsoever — the official Rust Reference even uses `hello-world` → `extern crate hello_world;` as its illustrative example.

### 1.2 What the ecosystem actually does (survey)

Two large, mature Rust+Tauri OSS projects were directly inspected:

- **`lapce/lapce`** (Tauri-adjacent code editor). Repo `lapce`, org `lapce`, workspace members `lapce-app`, `lapce-core`, `lapce-proxy`, `lapce-rpc` — all lowercase kebab-case with a project-prefix pattern. Branding in prose uses capitalized "Lapce", but every directory and package identifier is lowercase-hyphenated. Verified via GitHub API on the live default branch.
- **`spyglass-search/spyglass`** (Tauri 2.x desktop search). Repo `spyglass`, org `spyglass-search`. All top-level directories are lowercase single words (`apps`, `crates`, `plugins`, `utils`, `dockerfiles`, `fixtures`, `docs`, `scripts`, `assets`). Workspace members are all lowercase kebab-case (`spyglass-lens`, `spyglass-llm`, `spyglass-processor`, `spyglass-rpc`, `spyglass-searcher`, etc.). The Tauri-app member uses Tauri 2.x (`tauri-build = { version = "2", … }`).

The dominant pattern across both repos is: **lowercase kebab-case for all machine identifiers; capitalized brand for human-facing text**. The Rust API Guidelines does *not* prescribe this — it leaves crate casing as `unclear` — but the empirical convention is unambiguous.

### 1.3 Display name and reverse-DNS bundle id

The display name ("NeuralPitch") is independent of the package name. Tauri's `productName` field in `tauri.conf.json` and the bundle identifier (`com.<org>.<app>`) are conventionally set independently of the Cargo package. Reverse-DNS bundle identifiers are typically all lowercase: `com.<org>.neuralpitch` is normal; mixed-case in bundle ids is rare and historically caused issues on case-sensitive macOS bundle paths. (This last point is convention rather than a verified specific rule and is flagged as medium-confidence.)

### 1.4 Recommendation

| Element | Value | Reason |
|---|---|---|
| GitHub repo | `neural-pitch` | Matches Rust ecosystem norm; matches `lapce`, `spyglass`, almost every popular Rust project. |
| Cargo package name (root, if any) | `neural-pitch` | Identical to repo, frictionless `crates.io` publish path. |
| Workspace member crates | `neural-pitch-core`, `neural-pitch-io`, `neural-pitch-ml`, `neural-pitch-app` (or `neural-pitch` for the Tauri binary) | Mirrors lapce's `lapce-app`/`lapce-core`/… pattern; verified at primary source. |
| Tauri lib name | `neural_pitch_lib` (suffix `_lib`) | Tauri's official scaffolder hardcodes `let lib_name = format!("{}_lib", package_name.replace('-', "_"));` to dodge Windows bin/lib collision (rust-lang/cargo#8519). |
| Display name (README, Tauri `productName`) | `NeuralPitch` | Brand identity; ecosystem precedent (Lapce → `lapce`). |
| Bundle identifier | `com.<your-org>.neuralpitch` | All lowercase per common iOS/macOS convention. |

---

## 2. Repository / Project Structure

### 2.1 Tauri 2.x canonical layout

The Tauri 2.x official documentation ([v2.tauri.app/start/project-structure](https://v2.tauri.app/start/project-structure/)) states verbatim: *"the JavaScript project is at the top level, and the Rust project is inside `src-tauri/`."* Inside `src-tauri/` you find `Cargo.toml`, `build.rs`, `tauri.conf.json`, `src/{main.rs,lib.rs}`, `icons/`, and `capabilities/` (the Tauri 2.x ACL system; `permissions/` appears when you author custom permissions).

The same docs explicitly support two non-default modes for Rust-heavy projects:

> "If you want to work with Rust code only, simply remove everything else and use the `src-tauri/` folder as your top level project or as a member of your Rust workspace."

**This means a top-level Cargo workspace with `src-tauri/` as a member is officially blessed.**

### 2.2 lib.rs vs main.rs (mobile-critical)

Tauri 2.x compiles to a **library** for iOS/Android. The official docs state: *"we compile your app to a library in mobile builds and load them through the platform frameworks"* and *"don't modify this file [main.rs], modify `lib.rs` instead."* The official template's `main.rs` is a one-liner:

```rust
fn main() { neural_pitch_lib::run() }
```

All app logic lives in `lib.rs`, where the entry function is annotated `#[cfg_attr(mobile, tauri::mobile_entry_point)]`. Since NeuralPitch targets mobile later, **all behavior must live in the library crate from day 1**, even if mobile builds are deferred.

### 2.3 Multi-crate split — is it premature?

Two strong primary-source data points on the official Tauri repo (verified via GitHub API on the `dev` branch):

1. **`examples/api/src-tauri/Cargo.toml`** declares `crate-type = ["staticlib", "cdylib", "rlib"]` (staticlib for iOS, cdylib for Android, rlib for Rust-library consumption). It also declares `[lib] name = "api_lib"` (the `_lib` suffix convention). Most importantly, it consumes a **nested Rust crate** via path dependency: `tauri-plugin-sample = { path = "./tauri-plugin-sample/" }`. So Tauri's own example demonstrates that splitting Rust code into multiple crates inside a Tauri app is a first-class, framework-supported pattern.

2. **`spyglass`** is organized as a workspace with `apps/tauri/` for the Tauri binary and `crates/` for nine internal crates (`spyglass`, `spyglass-lens`, `spyglass-llm`, `spyglass-processor`, `spyglass-rpc`, `spyglass-searcher`, `entities`, `migrations`, `shared`). This is a real-world production multi-crate Tauri 2.x layout.

**Conclusion: a multi-crate split is *not* premature for a project that already plans desktop + iOS + Android + DSP + ML + audio I/O.** The platform abstraction boundaries you'll need (audio I/O differs sharply across desktop/iOS/Android; ML inference may be feature-gated by backend) map naturally onto crate boundaries.

### 2.4 Recommended layout

```
neural-pitch/                                # repo root
├─ Cargo.toml                                # [workspace], resolver = "2"
├─ Cargo.lock
├─ rust-toolchain.toml
├─ rustfmt.toml
├─ deny.toml
├─ .editorconfig
├─ .gitignore                                # Rust + Node + Tauri patterns
├─ README.md
├─ LICENSE-MIT
├─ LICENSE-APACHE
├─ CHANGELOG.md
├─ CONTRIBUTING.md
├─ SECURITY.md
├─ CODE_OF_CONDUCT.md                        # optional day-1
├─ index.html                                # web frontend at root (Tauri canonical)
├─ package.json
├─ vite.config.ts
├─ src/                                      # web frontend source
├─ public/
├─ dist/                                     # build output, gitignored
├─ src-tauri/                                # Tauri Rust app (workspace member)
│  ├─ Cargo.toml                             # crate-type = ["staticlib","cdylib","rlib"]
│  │                                         # [lib] name = "neural_pitch_lib"
│  ├─ build.rs
│  ├─ tauri.conf.json
│  ├─ src/
│  │  ├─ lib.rs                              # all logic here
│  │  └─ main.rs                             # thin shim — do not modify
│  ├─ capabilities/
│  ├─ permissions/                           # only if custom permissions
│  ├─ icons/
│  └─ gen/                                   # mobile, generated
├─ crates/                                   # workspace member crates (per spyglass convention)
│  ├─ neural-pitch-core/                     # DSP, pitch detection algorithms
│  │  ├─ Cargo.toml
│  │  ├─ src/lib.rs
│  │  ├─ benches/
│  │  └─ tests/
│  ├─ neural-pitch-io/                       # audio I/O abstractions, cpal/oboe/coreaudio
│  └─ neural-pitch-ml/                       # ONNX/Candle/Burn inference
├─ models/                                   # ML weights — gitignored; document fetch in scripts/
├─ assets/                                   # static, version-controlled assets
├─ docs/
│  ├─ adr/                                   # architecture decision records
│  └─ research/                              # this report lives here
├─ examples/                                 # cargo --example targets (top-level not under a crate)
├─ scripts/
│  └─ fetch-models.sh
└─ .github/
   ├─ workflows/
   │  ├─ ci.yml
   │  └─ release.yml
   ├─ ISSUE_TEMPLATE/
   └─ PULL_REQUEST_TEMPLATE.md
```

**Citations for layout choices:**
- Frontend at root, `src-tauri/` for Rust: Tauri 2.x project structure docs (primary).
- `src-tauri/` as workspace member: Tauri docs explicitly endorse this.
- `crates/` directory for workspace members: Spyglass primary repo + Cargo convention.
- `apps/<binary>/` split (alternative): also valid; Spyglass uses `apps/tauri/`.
- Three crate-types and `_lib` suffix: Tauri `examples/api/src-tauri/Cargo.toml` (verified verbatim).
- Lapce (`lapce-app`, `lapce-core`, `lapce-proxy`, `lapce-rpc`) for the kebab-case prefix-naming pattern.

### 2.5 ML weights: gitignored, fetched at runtime

`models/` should be git-ignored. Three patterns are common; choose based on size and license of weights:

1. **Hugging Face Hub at runtime** — `hf_hub` crate fetches on first use, caches under platform cache dir.
2. **Git LFS** — works but inflates clone size and counts against GitHub LFS quota.
3. **Bundled in Tauri resources** — only viable for small (< ~10 MB) weights since they ship in every binary.

This subsection is medium-confidence — choice depends on weight size and licensing terms not yet fixed. Defer the decision; gitignore the directory now.

---

## 3. Linux-Kernel Coding Standards Adapted for Rust

This section is **medium-confidence overall** — the verified-claim corpus did not include direct primary-source extracts from `Documentation/process/submitting-patches.rst` or `Documentation/rust/coding-guidelines.rst`. The recommendations below are best-practice synthesis grounded in widely-known kernel norms; readers should validate against the canonical kernel docs at <https://www.kernel.org/doc/html/latest/process/> and <https://www.kernel.org/doc/html/latest/rust/coding-guidelines.html> before formal adoption.

### 3.1 Commit message format (kernel-style)

The kernel norm for `Documentation/process/submitting-patches.rst`:
- Subject line: short (≤ ~72 chars), imperative mood, prefixed with subsystem area: `subsys: short imperative summary`.
- Blank line.
- Body: explain **why**, not what. Reference the problem being solved.
- Blank line.
- Trailers: `Signed-off-by:` (DCO), optionally `Reviewed-by:`, `Tested-by:`, `Fixes: <12-char-sha> ("<subject>")`, `Reported-by:`, `Co-developed-by:`.

For NeuralPitch, a **hybrid kernel + Conventional Commits** policy is pragmatic:

```
core: add YIN pitch detector with autocorrelation backend

The existing FFT-based estimator drifts at low frequencies because
spectral leakage dominates below ~80 Hz. YIN avoids this by working
in the time domain with a normalized cumulative-mean difference.

Benchmarks on the MIR-1K corpus show 4.2% lower MAE on bass clips.

Signed-off-by: Your Name <you@example.com>
```

The subsystem prefix (`core:`, `ui:`, `ci:`, `docs:`) is kernel-flavored; the imperative subject and "explain why" body align with both kernel and Conventional Commits norms. Conventional Commits' strict `type(scope): subject` (`feat:`, `fix:`, `chore:`) is *not* required; many high-quality Rust projects (rust-analyzer, ripgrep, lapce) use kernel-style subsystem prefixes, not Conventional Commits.

### 3.2 DCO sign-off

`git commit -s` appends `Signed-off-by: Name <email>`. Pros: legal clarity for OSS contributions; matches kernel and many projects (e.g., Docker, GitLab). Cons: minor friction for one-line typo fixes; some projects find it ceremonial overkill for personal projects.

**Recommendation: enable DCO sign-off from day 1.** `git config alias.cs 'commit -s'` or set `commit.gpgsign` + a hook. Cost is one keystroke; benefit is irreversible legal hygiene if/when contributors arrive.

### 3.3 Trailers worth borrowing

- `Fixes: <sha> ("<subject>")` — extremely useful for changelog generation and bisecting; cheap to write; **adopt**.
- `Reviewed-by:` / `Tested-by:` — primarily useful when multiple reviewers exist; defer until project has co-maintainers.
- `Reported-by:` — useful when the commit closes an issue raised by an external reporter.
- `Co-developed-by:` — only if pair-programmed.

### 3.4 "Don't add features you might need later"

The kernel maxim ("YAGNI" in kernel parlance) is doubly important in a multi-crate workspace: every premature crate boundary is a refactoring tax. **Recommendation: start with `neural-pitch-core` + `src-tauri/` only; split `io` and `ml` into separate crates only when concrete need arises** (mobile audio backends, alternate ML inference engine, etc.). This contradicts §2.4's full-fan-out layout — the realistic path is to *plan* the layout but only *create* the crates as needed.

---

## 4. Idiomatic Rust Conventions

### 4.1 Edition

Cargo's manifest reference (primary, verified): *"By default `cargo new` creates a manifest with the 2024 edition currently."* The 2024 edition was stabilized in Rust 1.85 (Feb 2025). **Use `edition = "2024"`** in every crate.

### 4.2 MSRV

Declare via `rust-version = "1.85"` (or whatever your floor is) in `[package]`. Tools like `cargo-msrv` (run periodically) and `cargo-check-rustc-min-version` validate it. For a brand-new project with no consumers, the cheapest policy is: track latest stable, advertise `rust-version = "<latest stable - 2 releases>"` only if/when you publish to crates.io. (Medium confidence — no primary-source quote was verified for MSRV-tooling guidance in the supplied claim corpus.)

### 4.3 License — the verified Rust dual-license pattern

The Cargo Manifest reference (primary, verified): the `license` field is *"an SPDX 2.3 license expression"* supporting `AND`, `OR`, and `WITH`. The reference's primary worked example is `license = "MIT OR Apache-2.0"`.

**Recommendation: `license = "MIT OR Apache-2.0"`** — the canonical Rust dual-license. Add both `LICENSE-MIT` and `LICENSE-APACHE` files at repo root. This matches `rust-lang/rust` itself and the vast majority of crates.io packages. Alternative `MPL-2.0` (file-level copyleft, similar to LGPL but cleaner) is reasonable if you want strong reciprocity per-file; it's used by Servo/Mozilla projects but is an uncommon choice in mainstream Rust.

### 4.4 SPDX file headers

The kernel-style `// SPDX-License-Identifier: MIT OR Apache-2.0` per-file header is **mandatory in the Linux kernel** but **not idiomatic in most Rust crates**. Most rust-lang/* and tokio-rs/* crates rely solely on the `license` field in `Cargo.toml` plus root `LICENSE-*` files; per-file headers are absent. (This claim is medium-confidence — the verified corpus did not include a survey of license-header prevalence.)

**Recommendation: skip per-file SPDX headers.** Trust `Cargo.toml`'s `license` field plus root `LICENSE-*` files; this matches Rust ecosystem norm.

### 4.5 rustfmt

Use `rustfmt.toml` minimally; nightly-only options reduce portability. A reasonable starter:

```toml
edition = "2024"
max_width = 100
```

(Spyglass repo has a `rustfmt.toml` at root — verified — but exact contents not extracted.)

### 4.6 Clippy

Recommended policy:
- `#![warn(clippy::all)]` and `#![warn(clippy::pedantic)]` at crate root, with targeted `#[allow(…)]` for noisy lints.
- CI runs `cargo clippy --all-targets --all-features -- -D warnings`.
- `clippy::pedantic` is reasonable for a new project; turn off later if signal-to-noise suffers. (Medium confidence — no primary-source claim verified.)

### 4.7 GitHub Actions CI baseline

A minimal idiomatic Rust CI:

```yaml
name: CI
on: [push, pull_request]
jobs:
  fmt:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { components: rustfmt }
      - run: cargo fmt --all -- --check
  clippy:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@stable
        with: { components: clippy }
      - run: cargo clippy --all-targets --all-features -- -D warnings
  test:
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest, windows-latest]
        toolchain: [stable, beta]
    runs-on: ${{ matrix.os }}
    steps:
      - uses: actions/checkout@v4
      - uses: dtolnay/rust-toolchain@v1
        with: { toolchain: ${{ matrix.toolchain }} }
      - run: cargo test --all-features
```

Tauri-specific: add a `tauri-build` job that runs on all three OSes to catch platform-specific build breakage.

### 4.8 Day-1 docs

| File | Day 1? | Reason |
|---|---|---|
| `README.md` | Yes | Required. |
| `LICENSE-MIT`, `LICENSE-APACHE` | Yes | Dual-license requires both. |
| `CHANGELOG.md` | Yes | Keep a Changelog format; trivial to add later but easier to keep current. |
| `CONTRIBUTING.md` | Yes (short) | Sets DCO + commit-format expectations. |
| `SECURITY.md` | Recommended | One paragraph: how to report security issues. |
| `CODE_OF_CONDUCT.md` | Optional | Adopt `contributor-covenant.org` text if/when contributors arrive. |
| `.editorconfig` | Yes | Keep editor whitespace consistent. |
| `deny.toml` | Recommended | `cargo-deny` for license + advisory + ban policies. |
| `rust-toolchain.toml` | Yes | Pins toolchain for reproducible CI. |

---

## 5. Adversarial Verification

For each headline recommendation, the dissenting position was considered:

| Recommendation | Strongest Counter | Verdict |
|---|---|---|
| Repo name `neural-pitch` (kebab) | "GitHub repos are case-insensitive in URLs but case-preserving — `NeuralPitch` looks better in titles." | **Holds.** GitHub treats `neural-pitch` and `Neural-Pitch` URLs as equivalent (case-insensitive lookup), but display preserves repo name casing. The Rust ecosystem norm (lapce, spyglass, ripgrep, tokio, serde, regex) is overwhelmingly lowercase kebab. The display brand can still be "NeuralPitch" in README. |
| Cargo package name `neural-pitch` | "Hyphens force you to write `neural_pitch` in Rust code — annoying." | **Mostly holds, with caveat.** The hyphen→underscore translation is automatic (RFC 940, Rust Reference verbatim) and uncontroversial in the ecosystem. If you find the friction annoying, name the crate `neural_pitch` directly — Cargo allows underscores too with no stated preference. Practical impact is nil either way. |
| Multi-crate workspace from day 1 | "YAGNI — you'll regret the boundaries when you need to refactor." | **Partly concedes.** Section 3.4 already softens the recommendation: plan the layout, but only create `crates/neural-pitch-core/` + `src-tauri/` initially. Add `io` and `ml` crates when you have a concrete need (e.g., mobile audio backend, alternate ML engine). |
| `MIT OR Apache-2.0` dual-license | "MPL-2.0 gives stronger copyleft for ML code without GPL viral effects." | **Defensible alternative.** MPL-2.0 is reasonable for a project that wants file-level copyleft. But the Rust ecosystem default is dual MIT/Apache; using anything else costs you compatibility with the path of least resistance. Stick with `MIT OR Apache-2.0` unless you have a specific reason. |
| Kernel-style commits (subsystem prefix, sign-off) | "Conventional Commits enables auto-changelogs and semantic-release tooling — kernel style breaks all that." | **Partly concedes.** If you want auto-changelog tooling (`git-cliff`, `release-please`), Conventional Commits is the path of least resistance. Hybrid is possible (kernel-style body + Conventional-Commits-compatible subject `feat(core): …`); pick one and be consistent. |
| DCO sign-off | "Personal projects don't need legal ceremony." | **Defer to user.** Real argument exists. The kernel uses DCO because of distributed contributor liability concerns; a solo project doesn't have that. Cost is one flag; benefit accrues only if contributors arrive. Lean toward enabling but acknowledge it's optional. |
| Edition 2024 | "Some dependencies don't yet support 2024." | **Holds.** 2024 is now the `cargo new` default per Cargo Manifest reference (verified). Rust editions are interop-compatible (a 2024-edition crate can depend on 2018/2021-edition crates). No real blocker. |

---

## 6. Decisions Recommended

| Decision | Value |
|---|---|
| GitHub repo name | `neural-pitch` |
| Cargo package name (Tauri app) | `neural-pitch` |
| Tauri lib name | `neural_pitch_lib` (`_lib` suffix per Tauri scaffolder convention) |
| Workspace member crates (planned) | `neural-pitch-core`, `neural-pitch-io`, `neural-pitch-ml` (create incrementally) |
| Tauri lib `crate-type` | `["staticlib", "cdylib", "rlib"]` (mobile + desktop + Rust-lib) |
| Workspace layout | Top-level Cargo workspace; `src-tauri/` as workspace member; web frontend at repo root; `crates/` directory for non-Tauri members |
| Display name (README, `productName`, app title) | `NeuralPitch` |
| Bundle identifier | `com.<your-org>.neuralpitch` (lowercase) |
| License | `MIT OR Apache-2.0` (SPDX expression, both `LICENSE-*` files at root) |
| SPDX per-file headers | Skip — not idiomatic in Rust ecosystem |
| Edition | `2024` (current `cargo new` default, verified) |
| MSRV | `rust-version = "1.85"` (Rust 2024-edition floor); revisit if/when published to crates.io |
| Commit message policy | Kernel-style hybrid: `subsys: imperative subject` + body explaining "why" + DCO sign-off. Consider Conventional Commits if/when auto-changelog tooling is adopted. |
| Sign-off policy | DCO `Signed-off-by:` enabled (`git commit -s`) from day 1 |
| `Fixes:` trailer | Adopt — cheap and useful |
| `Reviewed-by:` / `Tested-by:` trailers | Defer until project has co-maintainers |
| CI baseline | GitHub Actions: fmt + clippy + test (stable + beta on Linux/macOS/Windows); Tauri-specific build job |
| Clippy policy | `warn(clippy::all)` + `warn(clippy::pedantic)` at crate root; CI denies warnings |
| Day-1 files | README, dual LICENSE, CHANGELOG, CONTRIBUTING, SECURITY, .editorconfig, .gitignore, rust-toolchain.toml, deny.toml |
| `models/` | Gitignored; fetch via `hf_hub` at runtime (revisit when weight size is fixed) |

---

## Open Questions

1. **`apps/` vs `crates/` for the Tauri binary.** Spyglass uses `apps/tauri/`; lapce uses flat top-level. Which is preferred for a single-Tauri-app + several library crates project? Verified evidence is split.
2. **MSRV cadence.** No verified primary-source guidance on Rust ecosystem MSRV norms (e.g., "latest stable" vs "stable - 6 months" vs "MSRV declared per-crate"). Worth a follow-up.
3. **Per-file SPDX headers prevalence on crates.io.** No quantitative survey was verified. Decision was made on convention rather than measurement.
4. **`hf_hub` vs Git LFS vs bundled-resource for model weights.** Decision deferred to weight-size and license phase; needs revisiting when ML backend is chosen.

---

## Sources (Primary)

- Cargo Reference, *The Manifest Format*: <https://doc.rust-lang.org/cargo/reference/manifest.html>
- Cargo Reference, *Cargo Targets*: <https://doc.rust-lang.org/cargo/reference/cargo-targets.html>
- Rust Reference, *Extern Crates*: <https://doc.rust-lang.org/reference/items/extern-crates.html>
- Rust Reference, *Identifiers*: <https://doc.rust-lang.org/reference/identifiers.html>
- Rust API Guidelines, *Naming*: <https://rust-lang.github.io/api-guidelines/naming.html>
- RFC 940 — *Hyphens Considered Harmful*: <https://github.com/rust-lang/rfcs/blob/master/text/0940-hyphens-considered-harmful.md>
- Tauri 2.x Docs, *Project Structure*: <https://v2.tauri.app/start/project-structure/>
- Tauri 2.x Docs, *Create a Project*: <https://v2.tauri.app/start/create-project/>
- Tauri 2.x Docs, *Migrate from Tauri 1*: <https://v2.tauri.app/start/migrate/from-tauri-1/>
- Tauri Repo, *examples/api/src-tauri/Cargo.toml*: <https://github.com/tauri-apps/tauri/blob/dev/examples/api/src-tauri/Cargo.toml>
- create-tauri-app, *base template Cargo.toml*: <https://github.com/tauri-apps/create-tauri-app>
- Lapce repo: <https://github.com/lapce/lapce>
- Spyglass repo: <https://github.com/spyglass-search/spyglass>
- Linux kernel, *Submitting Patches*: <https://www.kernel.org/doc/html/latest/process/submitting-patches.html> (cited but not directly verified in claim corpus)
- Linux kernel, *Rust coding guidelines*: <https://www.kernel.org/doc/html/latest/rust/coding-guidelines.html> (cited but not directly verified in claim corpus)
