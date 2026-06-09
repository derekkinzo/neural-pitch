//! Convenience re-exports for downstream callers.
//!
//! `use neural_pitch_core::prelude::*;` brings in the most commonly used types
//! across the pitch detection, music theory, audio capture, and pipeline
//! modules.

#[cfg(feature = "cpal")]
pub use crate::audio::CpalAudioBackend;
pub use crate::audio::{
    AudioBackend, AudioBackendConfig, AudioBackendError, AudioBackendEvent, MockAudioBackend,
    Pacing, SampleSource,
};
pub use crate::music::{NoteReading, frequency_to_note, midi_to_hz};
pub use crate::pipeline::{
    ChannelFrameSink, DspError, DspWorker, FrameSink, FrameSinkError, PitchUpdate,
};
pub use crate::pitch::{EstimatorConfig, EstimatorError, F0Frame, InstrumentHint, PitchEstimator};
pub use crate::settings::{SETTINGS_SCHEMA_VERSION, SettingsError, TunerSettings, migrate};
#[cfg(feature = "neural")]
pub use crate::stems::{StemError, StemResult, StemSeparator};
pub use crate::store::{
    ListFilter, NewRecording, Recording, RecordingId, RecordingsLibrary, StoreError,
};
