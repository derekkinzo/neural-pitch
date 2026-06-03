# ADR-0002: Mobile-ready repo and crate shape day 1

## Status

Accepted — 2026-06-02.

## Context

Phase 6 of the roadmap adds Tauri Mobile (iOS + Android) builds. Tauri Mobile imposes specific build-shape requirements that, if retrofitted later, would force a disruptive refactor: the library crate must be named with a snake_case `_lib` suffix, must emit `staticlib` (iOS), `cdylib` (Android), and `rlib` (desktop and tests), and must keep all entry-point logic in `lib.rs` so that mobile entry points (which never link `main.rs`) can still reach it.

A pure-Rust core (no Tauri imports) is also required so that the same DSP code can be reused by a future CLI, by tests, and by mobile shells without modification.

## Decision

From the first commit:

- The Tauri lib is named `neural_pitch_lib`.
- `crate-type = ["staticlib", "cdylib", "rlib"]` in `src-tauri/Cargo.toml`.
- All Tauri builder logic lives in `src-tauri/src/lib.rs` under `#[cfg_attr(mobile, tauri::mobile_entry_point)] pub fn run()`.
- `src-tauri/src/main.rs` is a one-line shim: `fn main() { neural_pitch_lib::run() }`.
- The pure-Rust core lives in `crates/neural-pitch-core/` and contains no `tauri::*` or `tauri_plugin_*` imports.
- Audio I/O is abstracted behind traits (`PitchEstimator`, `FrameSink`, future `AudioBackend`) so future mobile audio backends (oboe on Android, AVAudioEngine on iOS) can substitute without core changes.
- Bundle ID is `com.<org>.neuralpitch` (lowercase), matching Apple/Google conventions.
- Edition 2024, MSRV 1.88 (bumped from 1.85 to consume `time` 0.3.47, which patches RUSTSEC-2026-0009).

No mobile builds are produced until Phase 6.

## Consequences

- Phase 0 contributors must understand and respect the lib/main split even before any mobile work begins.
- The pure-Rust-core invariant is enforced by code review and a CI check that greps for `tauri::` in `crates/`.
- Mobile entry retrofitting at Phase 6 becomes mechanical rather than disruptive.
- The `crate-type = ["staticlib", "cdylib", "rlib"]` widening produces slightly larger build outputs even on desktop; the cost is acceptable for the simplification at Phase 6.

## Alternatives Considered

- **Defer mobile shape to Phase 6** — rejected because the lib/main split and the pure-Rust-core invariant are touched by every phase between 1 and 5; retrofitting them later would force a sweeping diff across stable code.
- **Maintain a separate mobile branch** — rejected because branches diverge; the project is small enough that a single trunk with mobile-shaped scaffolding is cheaper than maintaining a fork.
- **Use `cargo-mobile2`'s default skeleton** — rejected because that skeleton encodes choices (state shape, IPC patterns) that conflict with the locked Tauri 2.x conventions.
