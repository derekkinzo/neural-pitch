# NeuralPitch

A modular, FOSS pitch-detection app for singers and musicians.

[![CI](https://img.shields.io/badge/CI-pending-lightgrey.svg)](#)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue.svg)](#license)

## About

NeuralPitch is a real-time pitch-detection application targeted at singers
who want to learn and explore their vocal range. It is built as a modular
core so that future features such as song analysis, polyphonic transcription,
and stem separation can be layered on without rewriting the foundation. The
desktop app is implemented as a Rust core wrapped in a Tauri 2.x shell, with
mobile support as an aspirational follow-on.

## Status

Phase 0 (skeleton). The repository is being scaffolded; no functional pitch
detection is wired up yet. See
[docs/design/DESIGN.md §13](docs/design/DESIGN.md) for the phased roadmap.

## Documentation

- [docs/design/DESIGN.md](docs/design/DESIGN.md) — authoritative design doc.
- [docs/adr/README.md](docs/adr/README.md) — index of architecture decision records.
- [docs/research/RESEARCH-REPORT.md](docs/research/RESEARCH-REPORT.md) — primary research report.
- [docs/research/MODULAR-PITCH-RESEARCH.md](docs/research/MODULAR-PITCH-RESEARCH.md) — modular pitch-detection research.
- [docs/research/REPO-CONVENTIONS-REPORT.md](docs/research/REPO-CONVENTIONS-REPORT.md) — repository-conventions research.

## Building

Prerequisites (placeholder — fuller instructions land with Phase 1):

- [rustup](https://rustup.rs/) (toolchain version is pinned by `rust-toolchain.toml`).
- Node.js 20 or newer.
- `pnpm` (preferred) or `npm`.
- [`pre-commit`](https://pre-commit.com/) — `pre-commit install` is **required** before your first commit.

### Linux system libraries

Tauri 2.x requires GTK / WebKitGTK on Linux. On Debian / Ubuntu:

```sh
sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev \
    libayatana-appindicator3-dev librsvg2-dev libsoup-3.0-dev pkg-config
```

If you only intend to work on the pure-Rust core (`crates/neural-pitch-core`),
these are not required — `cargo test -p neural-pitch-core` runs without them.

### Building with neural support

The neural pitch backends — `PestoEstimator`, `CrepeTinyEstimator`, and the
shared Viterbi decoder — are **off by default** and gated behind the
`neural` Cargo feature on `crates/neural-pitch-core`. The default build
ships only the classical YIN/MPM (and optional pYIN) estimators, has zero
ONNX runtime dependency, and stays purely under `MIT OR Apache-2.0`. See
[ADR-0020](docs/adr/0020-neural-feature-gate.md) for why the feature gate
exists and why the ONNX weights are treated as runtime assets rather than
bundled artifacts.

To opt in:

```sh
# 1. Build with the neural feature on the core crate.
cargo build -p neural-pitch-core --features neural

# 2. Fetch the ONNX weights (PESTO and/or CREPE-tiny) into ./models/.
#    Weights are NOT checked in — they are resolved by models.toml.
./scripts/fetch-models.sh
```

PESTO's inference repository is LGPL-3.0 and the redistributable status of
the weights is contested; opting in moves that question to your build site.
CREPE-tiny is the MIT-licensed fallback if you want to avoid the LGPL
question entirely.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for commit conventions, DCO sign-off,
hooks, and CI expectations.

## License

Dual-licensed under MIT OR Apache-2.0 at your option. See
[LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
