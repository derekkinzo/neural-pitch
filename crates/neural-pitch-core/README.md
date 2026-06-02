# neural-pitch-core

Pure-Rust core library for the NeuralPitch project. Provides the canonical
`PitchEstimator` trait, music-theory math (frequency-to-note, MIDI conversion,
cents), audio I/O abstractions, contour smoothing, and a voice-activity gate.
This crate is the single unit of reuse across the desktop app, future CLI,
and a possible mobile shell — it has no Tauri imports and is platform-portable.

See [docs/design/DESIGN.md §5](../../docs/design/DESIGN.md) for the authoritative
module layout, public API, and crate-level invariants.
