# neural-pitch-core

Pure-Rust core library for NeuralPitch. Provides:

- `PitchEstimator` trait and YIN/MPM, pYIN, PESTO, and CREPE-tiny backends.
- Music-theory math (frequency-to-note, MIDI conversion, cents).
- Audio I/O abstractions over CPAL with mockable backends.
- DSP pipeline worker, contour smoothing, and a voice-activity gate.
- SQLite-backed recordings library and analysis cache.
- Range and vibrato analysis over offline contours.

The crate has no Tauri imports and is platform-portable. Neural backends
(PESTO, CREPE-tiny) are gated behind the `neural` Cargo feature.
