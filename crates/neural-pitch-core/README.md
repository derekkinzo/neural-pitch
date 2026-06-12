# neural-pitch-core

Pure-Rust core library for NeuralPitch. Provides:

- `PitchEstimator` trait with YIN/MPM and pYIN backends. (A
  CREPE-tiny stub estimator exists for in-tree tests only; the
  shipping offline pitch path is pYIN.)
- `PolyEstimator` trait and a Basic Pitch v1 backend
  (Bittner et al., ICASSP 2022) for polyphonic transcription.
- HTDemucs (Defossez 2021) four-bus stem separation with on-disk model
  caching and SHA-256 pinning.
- Music-theory math (frequency-to-note, MIDI conversion, cents).
- Audio I/O abstractions over CPAL with mockable backends.
- DSP pipeline worker, contour smoothing, and a voice-activity gate.
- SQLite-backed recordings library and analysis cache.
- Range, vibrato, and ear-training drill kernels over offline contours.

The crate has no Tauri imports and is platform-portable. The neural
Basic Pitch and HTDemucs backends are gated behind the `neural` Cargo
feature; the default build stays under MIT OR Apache-2.0.
