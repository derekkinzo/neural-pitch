//! Ear-training core.
//!
//! Pure-Rust music-theory primitives that power the drill subsystem:
//! intervals, chord-quality classification, scale/mode classification,
//! solfege (movable & fixed do), drill specs/results, and a real-time
//! `TargetMatcher` that consumes [`crate::pipeline::PitchUpdate`] frames
//! and emits scoring summaries for the karaoke ribbon and drill UIs.
//!
//! Feature-gate-free; ships in every build configuration.
//! The training module intentionally consumes only
//! `music::frequency_to_note`, `music::midi_to_hz`, and
//! `pipeline::sink::PitchUpdate` from the rest of the crate — it is a
//! leaf module and never imports `pitch::*`, `audio::*`, or `store::*`.

pub mod chords;
pub mod drill;
pub mod intervals;
// IPC layer (start_drill_blocking, submit_drill_attempt_blocking,
// list_drill_history_blocking, synthesize_prompt_blocking) used by
// `src-tauri/src/commands_drill.rs`. Distinct from the algorithm-side
// types in `drill.rs`; the IPC structs use the `Ipc*` prefix to avoid
// the name collision.
pub mod ipc;
pub mod scales;
pub mod solfege;
pub mod target_match;

pub use chords::ChordQuality;
pub use drill::{Drill, DrillResult, DrillSpec, HitWindow};
pub use intervals::Interval;
pub use ipc::{
    AttemptPayload, DrillAttempt, DrillError, DrillKind, DrillSession, DrillSessionId,
    HISTORY_LIMIT_CAP, HistoryFilter, IpcDrillResult, IpcDrillSpec, NoteSpec, SESSION_LRU_CAPACITY,
    list_drill_history_blocking, start_drill_blocking, submit_drill_attempt_blocking,
    synthesize_prompt_blocking,
};
pub use scales::ScaleMode;
pub use solfege::{Accidental, Direction, KeyMode, MinorMode, Note, NoteName, SolfegeSyllable};
pub use target_match::{MatchUpdate, TargetMatcher};
