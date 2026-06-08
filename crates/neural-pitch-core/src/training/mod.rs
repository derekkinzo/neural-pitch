//! Phase 4 ear-training core.
//!
//! Pure-Rust music-theory primitives that power the drill subsystem:
//! intervals, chord-quality classification, scale/mode classification,
//! solfege (movable & fixed do), drill specs/results, and a real-time
//! `TargetMatcher` that consumes [`crate::pipeline::PitchUpdate`] frames
//! and emits scoring summaries for the karaoke ribbon and drill UIs.
//!
//! Default-on (no `feature = "neural"` gate) so both
//! `--no-default-features` and `--all-features` builds see this module.
//! The training module intentionally consumes only
//! `music::frequency_to_note`, `music::midi_to_hz`, and
//! `pipeline::sink::PitchUpdate` from the rest of the crate — it is a
//! leaf module and never imports `pitch::*`, `audio::*`, or `store::*`.
//!
//! Phase 4 RED stubs: every public item below is `todo!()`-bodied. The
//! corresponding integration tests under `tests/` exercise the public
//! surface and fail with `not yet implemented` panics until the GREEN
//! implementation lands.

pub mod chords;
pub mod drill;
pub mod intervals;
// Phase 4 IPC stub layer (start_drill_blocking, submit_drill_attempt_blocking,
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
