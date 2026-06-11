# NeuralPitch

A small, FOSS pitch-detection app for musicians and audio nerds. Desktop
build is a Rust core wrapped in a Tauri 2 shell with a React + Tailwind
front end. Dual-licensed under MIT OR Apache-2.0.

## What it does today

- **Live tuner.** Real-time pitch detection from the default input device
  — guitar, piano, voice, violin, bass, or anything else with a
  fundamental. Note name, cents-off meter, and recent-history strip update
  at audio rate. Settings (reference A, transpose, tuning system) persist
  across launches.
- **Recordings.** Capture input audio to FLAC, store it in a per-platform
  SQLite library, and rename or delete entries from the UI.
- **Offline analysis.** Run pYIN over a recording, smooth the contour, and
  cache the results in the library.
- **Range and vibrato reports.** Per-recording lowest/highest pitch,
  tessitura, and vibrato rate / extent / regularity, computed from the
  cached contour.
- **Import and transcribe.** Pick a WAV or FLAC file, run polyphonic
  transcription, and export the result as a Standard MIDI File.
- **Ear-training drills.** Intervals, chord quality, scale identification,
  pitch-matching with a karaoke ribbon, and tuning practice — with
  movable-do solfege as a display option.
- **Stem separation.** Split a recording into vocals, drums, bass, and
  other, play each stem, and run polyphonic transcription on any stem.
  The HTDemucs ONNX model (~316 MB on first use) is downloaded from
  huggingface.co/StemSplitio/htdemucs-onnx and pinned by SHA-256; if the
  model is unavailable and the host is offline, separation surfaces a
  typed error with the manual-download URL.

The `neural-pitch-core` crate is pure Rust, has no Tauri dependency, and
can be reused as a library. The default build pulls in only the classical
estimators (YIN / MPM / pYIN) and stays under MIT OR Apache-2.0.

## Building

You need:

- `rustup` (toolchain pinned by `rust-toolchain.toml`)
- Node.js 20+
- npm

On Debian / Ubuntu, Tauri also needs:

```sh
sudo apt-get install -y libwebkit2gtk-4.1-dev libgtk-3-dev \
    libayatana-appindicator3-dev librsvg2-dev libsoup-3.0-dev \
    libasound2-dev pkg-config
```

If you only want to hack on the core crate, none of the system libraries
are required — `cargo test -p neural-pitch-core` runs standalone.

### Run the desktop app

```sh
npm install
npm run build           # produces dist/, which Tauri's
                        # generate_context! reads at compile time
cargo tauri dev         # or: cargo run -p neural-pitch
```

### Build the core crate only

```sh
cargo build -p neural-pitch-core
```

## Running tests

The local CI gate mirrors the GitHub Actions workflow:

```sh
scripts/ci-local.sh           # quick tier (default), ~3 min warm cache
scripts/ci-local.sh visual    # + Playwright visual baselines
scripts/ci-local.sh full      # + `act` replay of every Linux CI job
```

The quick tier is the pre-push contract: if it passes locally, CI will
pass on push. It runs `cargo fmt`, clippy under both `--all-features` and
`--no-default-features`, the workspace test suite on stable (and beta if
you have it installed), `cargo deny`, the TypeScript and ESLint checks,
the Tauri release build, and the voice-acceptance harness.

If you just want to run the tests directly:

```sh
cargo test --workspace --all-features
cargo test -p neural-pitch-core --no-default-features
npm run typecheck && npm run lint
npx playwright test
```

### Running the ONNX-backed `#[ignore]`d tests

Basic Pitch and HTDemucs use `ort` with `load-dynamic`, which resolves
`libonnxruntime.so` at runtime via the system loader or `ORT_DYLIB_PATH`.
If neither path resolves, ONNX session construction blocks inside
`dlopen`. Point `ORT_DYLIB_PATH` at a working copy and include
`--include-ignored`:

```sh
export ORT_DYLIB_PATH=/path/to/libonnxruntime.so
cargo test --workspace --features neural -- --include-ignored
```

`scripts/ci-local.sh` auto-detects a few common cache paths so the
local gate's ONNX tests do not silently hang.

## License

Dual-licensed under MIT OR Apache-2.0 at your option.
See [LICENSE-MIT](LICENSE-MIT) and [LICENSE-APACHE](LICENSE-APACHE).
