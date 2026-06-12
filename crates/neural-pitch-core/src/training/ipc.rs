//! Ear-training drill subsystem IPC surface.
//!
//! This module owns the drill spec types, the persisted attempt row
//! shape, and the `*_blocking` helpers the Tauri shell wraps in
//! `spawn_blocking`. It mirrors the
//! [`crate::store::analyze_recording_blocking`] layout and the
//! [`neural_pitch_lib::transcribe`](../../../src-tauri/src/transcribe.rs)
//! layout so the IPC surface stays uniform across subsystems.
//!
//! # Behaviour
//!
//! 1. [`start_drill_blocking`] validates the [`IpcDrillSpec`], renders
//!    the prompt audio via the additive synth, mints a fresh
//!    [`DrillSessionId`], and returns the [`DrillSession`] handle. The
//!    helper itself is stateless — the in-flight session map kept by
//!    the Tauri shell on `AppState` is purely a cache for the page-side
//!    front-end's resume affordance and is not consulted by the scorer.
//! 2. [`submit_drill_attempt_blocking`] reduces the per-frame
//!    [`AttemptPayload`] to `(mean_cents_error, time_on_pitch_ratio)`,
//!    computes a per-kind percentile from `drill_attempts`, persists
//!    the row, and returns the [`IpcDrillResult`].
//! 3. [`list_drill_history_blocking`] pages over `drill_attempts`
//!    server-side clamped to [`HISTORY_LIMIT_CAP`] using the new
//!    `idx_drill_attempts_history` index for ordering.

use std::str::FromStr;

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::settings::DEFAULT_A4_HZ;
use crate::store::{NewDrillAttempt, RecordingId, RecordingsLibrary, StoreError};

/// Server-side cap on `HistoryFilter::limit`. Requests above this
/// clamp silently to 200 so a buggy front-end cannot starve the IPC
/// thread on a giant SELECT.
pub const HISTORY_LIMIT_CAP: u32 = 200;

/// Bounded LRU capacity for the in-memory drill-session map kept on
/// `AppState`. The session map only holds sessions that have not yet
/// been submitted, so 64 in flight is generous for any realistic drill
/// cadence. Exported so a future `AppState::drill_sessions` initialiser
/// picks up the same constant the IPC contract documents.
pub const SESSION_LRU_CAPACITY: usize = 64;

/// Stable handle to an in-flight drill session.
///
/// Backed by a UUIDv7 (16 bytes), same as
/// [`crate::store::RecordingId`], so the IPC boundary marshalling
/// re-uses the same hex string parser. Distinct type so the type
/// system catches accidental cross-wiring.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct DrillSessionId(pub [u8; 16]);

impl DrillSessionId {
    /// Mint a fresh UUIDv7-backed id.
    #[must_use]
    pub fn new_v7() -> Self {
        Self(*uuid::Uuid::now_v7().as_bytes())
    }
}

impl core::fmt::Display for DrillSessionId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}", uuid::Uuid::from_bytes(self.0))
    }
}

impl FromStr for DrillSessionId {
    type Err = uuid::Error;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(*uuid::Uuid::parse_str(s)?.as_bytes()))
    }
}

impl Serialize for DrillSessionId {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for DrillSessionId {
    fn deserialize<D: serde::Deserializer<'de>>(de: D) -> Result<Self, D::Error> {
        use serde::de::Error;
        let s = String::deserialize(de)?;
        DrillSessionId::from_str(&s).map_err(D::Error::custom)
    }
}

/// One musical note specification used by the prompt synth and the
/// scoring logic.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct NoteSpec {
    /// MIDI note number `0..=127`.
    pub midi: i32,
    /// Reference A4 pitch in Hertz. Defaults to 440.0 when callers do
    /// not specify; the live-tuner setting is the canonical source.
    pub a4_hz: f32,
}

impl NoteSpec {
    /// Construct a [`NoteSpec`] at the standard 440 Hz A4 reference.
    #[must_use]
    pub fn new(midi: i32) -> Self {
        Self {
            midi,
            a4_hz: DEFAULT_A4_HZ,
        }
    }
}

/// The kind of drill being run. Persisted as a free-form string in
/// `drill_attempts.drill_kind` so future drill kinds do not require a
/// schema migration.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum DrillKind {
    /// Two notes — caller picks one of the standard interval names.
    Interval,
    /// Sight-singing — caller is shown a phrase and sings it back.
    SightSing,
    /// Vocal-range drill — caller is asked to glide between two
    /// boundary notes for the range-aware UI.
    Range,
}

impl DrillKind {
    /// Stable wire string used in the `drill_kind` column. Matches the
    /// `serde(rename_all = "snake_case")` output but is exposed
    /// explicitly so the SQLite layer does not depend on serde for
    /// the round-trip.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Interval => "interval",
            Self::SightSing => "sight_sing",
            Self::Range => "range",
        }
    }
}

/// IPC-side spec passed to `start_drill`. The shell encodes this in the
/// session's `drill_payload` blob via postcard so the scoring logic
/// can re-derive the expected response without keeping the entire
/// session struct in the in-memory LRU.
///
/// Distinct from the algorithm-side
/// [`crate::training::DrillSpec`] (re-exported from
/// `training/drill.rs`). Both surfaces stay under their own names so
/// the IPC structs and the algorithm structs remain compilable side
/// by side without name collision.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IpcDrillSpec {
    /// Drill kind discriminator.
    pub kind: DrillKind,
    /// Prompt note(s) — at least one. Interval drills carry two;
    /// sight-singing carries the sequence; range drills carry the
    /// boundary pair.
    pub prompt_notes: Vec<NoteSpec>,
    /// Expected response from the user, in MIDI numbers. Scoring is
    /// against this list.
    pub expected_response_midi: Vec<i32>,
}

/// Session handle returned from `start_drill`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrillSession {
    /// Stable session id. Stringified at the IPC boundary.
    pub session_id: DrillSessionId,
    /// PCM16 mono RIFF/WAVE byte stream the front-end pipes into a
    /// `Blob` URL for `<audio>` playback.
    pub prompt_wav: Vec<u8>,
    /// First prompt note's MIDI number, surfaced as a convenience for
    /// the karaoke-ribbon scrubber.
    pub prompt_note_midi: i32,
    /// Expected first-response MIDI number. The front-end's matcher
    /// uses this to pre-arm the karaoke ribbon before the audio
    /// finishes playing.
    pub expected_response_midi: i32,
}

/// Payload submitted on attempt.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AttemptPayload {
    /// Per-frame cents-error samples captured during the user's
    /// response. The scorer reduces this to `mean_cents_error` and
    /// `time_on_pitch_ratio`.
    pub cents_error_frames: Vec<f32>,
    /// Per-frame voiced flags aligned with `cents_error_frames`.
    pub voiced_frames: Vec<bool>,
    /// Wall-clock attempt-start timestamp in Unix milliseconds.
    pub started_at_unix_ms: i64,
    /// Wall-clock attempt-finish timestamp in Unix milliseconds.
    pub finished_at_unix_ms: i64,
}

/// IPC-side outcome of one drill attempt. Distinct from the
/// algorithm-side [`crate::training::DrillResult`] (re-exported from
/// `training/drill.rs`); see [`IpcDrillSpec`] for the rationale.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct IpcDrillResult {
    /// `true` if the scorer judged the attempt correct against the
    /// session's `expected_response_midi`.
    pub correct: bool,
    /// Mean cents error across voiced frames. NaN if no voiced frames.
    pub mean_cents_error: f32,
    /// Fraction of voiced frames that were within the in-window
    /// tolerance. Range `[0.0, 1.0]`.
    pub time_on_pitch_ratio: f32,
    /// Percentile of `mean_cents_error` against the user's history of
    /// the same `DrillKind`. Range `[0.0, 1.0]`; lower = better.
    pub percentile_in_kind: f32,
}

/// Filter knob for `list_drill_history`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryFilter {
    /// Optional drill-kind filter. `None` = every kind.
    pub kind: Option<String>,
    /// Optional lower bound on `finished_at_unix_ms`.
    pub since_unix_ms: Option<i64>,
    /// Maximum rows to return. Server-side clamped to
    /// [`HISTORY_LIMIT_CAP`].
    pub limit: u32,
    /// Pagination offset.
    pub offset: u32,
}

impl Default for HistoryFilter {
    fn default() -> Self {
        Self {
            kind: None,
            since_unix_ms: None,
            limit: HISTORY_LIMIT_CAP,
            offset: 0,
        }
    }
}

/// One row in the `drill_attempts` table.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrillAttempt {
    /// UUIDv7 attempt id (stringified at the IPC boundary).
    pub id: String,
    /// Drill-kind discriminant string (matches [`DrillKind::as_str`]).
    pub drill_kind: String,
    /// Whether the scorer judged the attempt correct.
    pub correct: bool,
    /// Mean cents error across voiced frames.
    pub mean_cents_error: f32,
    /// Fraction of voiced frames in the in-window tolerance.
    pub time_on_pitch_ratio: f32,
    /// Wall-clock attempt-start timestamp in Unix milliseconds.
    pub started_at_unix_ms: i64,
    /// Wall-clock attempt-finish timestamp in Unix milliseconds.
    pub finished_at_unix_ms: i64,
    /// Optional recording-id this attempt was paired with.
    pub recording_id: Option<String>,
}

/// Errors returned by the drill blocking helpers.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum DrillError {
    /// Sentinel for code paths that have no implementation. Reserved
    /// `#[non_exhaustive]` slot so the IPC surface can grow without a
    /// breaking change; no helper returns this variant in production.
    #[error("drill subsystem path not implemented")]
    NotImplemented,
    /// SQLite library failure.
    #[error("library: {0}")]
    Library(String),
    /// Postcard encode / decode failure for the session payload blob.
    #[error("payload codec: {0}")]
    PayloadCodec(String),
    /// Caller supplied a session id that was not in the in-memory LRU.
    #[error("unknown session id: {0}")]
    UnknownSession(String),
    /// Caller-supplied IpcDrillSpec failed validation.
    #[error("invalid drill spec: {0}")]
    InvalidSpec(String),
}

impl From<StoreError> for DrillError {
    fn from(e: StoreError) -> Self {
        Self::Library(format!("{e:#}"))
    }
}

/// Default duration the prompt synth is asked for when
/// [`start_drill_blocking`] hands the spec's first prompt note to
/// [`crate::poly::synth::PromptSynth::render_wav_at_a4`]. 1.2 s covers
/// every interval / chord / sight-singing prompt; longer melodies are
/// rendered on a per-note basis by the front-end through
/// [`synthesize_prompt_blocking`].
const PROMPT_RENDER_MS: u32 = 1_200;

fn validate_spec(spec: &IpcDrillSpec) -> Result<(), DrillError> {
    if spec.prompt_notes.is_empty() {
        return Err(DrillError::InvalidSpec(
            "prompt_notes must contain at least one entry".into(),
        ));
    }
    if spec.expected_response_midi.is_empty() {
        return Err(DrillError::InvalidSpec(
            "expected_response_midi must contain at least one entry".into(),
        ));
    }
    for note in &spec.prompt_notes {
        if !(0..=127).contains(&note.midi) {
            return Err(DrillError::InvalidSpec(format!(
                "prompt midi {} out of range 0..=127",
                note.midi
            )));
        }
        if !note.a4_hz.is_finite() || note.a4_hz <= 0.0 {
            return Err(DrillError::InvalidSpec(format!(
                "prompt a4_hz must be finite and positive, got {}",
                note.a4_hz
            )));
        }
    }
    for &m in &spec.expected_response_midi {
        if !(0..=127).contains(&m) {
            return Err(DrillError::InvalidSpec(format!(
                "expected_response_midi {m} out of range 0..=127"
            )));
        }
    }
    Ok(())
}

/// Render the prompt audio via the additive synth.
fn render_prompt_wav(note: NoteSpec) -> Result<Vec<u8>, DrillError> {
    let mut synth = crate::poly::synth::PromptSynth::new();
    synth
        .render_wav_at_a4(note.midi, PROMPT_RENDER_MS, note.a4_hz)
        .map_err(|e| DrillError::Library(format!("{e:#}")))
}

/// Begin a new drill session. Pure-Rust headless twin the Tauri
/// `start_drill` command wraps in `spawn_blocking`.
///
/// # Errors
///
/// Returns [`DrillError::InvalidSpec`] when the supplied
/// [`IpcDrillSpec`] is empty or carries an out-of-range MIDI value;
/// [`DrillError::Library`] when the additive synth's WAV render
/// fails.
pub fn start_drill_blocking(spec: &IpcDrillSpec) -> Result<DrillSession, DrillError> {
    validate_spec(spec)?;
    let first_prompt = spec.prompt_notes[0];
    let prompt_wav = render_prompt_wav(first_prompt)?;
    Ok(DrillSession {
        session_id: DrillSessionId::new_v7(),
        prompt_wav,
        prompt_note_midi: first_prompt.midi,
        expected_response_midi: spec.expected_response_midi[0],
    })
}

/// Reduce per-frame cents-error samples to
/// `(mean_cents_error, time_on_pitch_ratio)` over voiced frames.
fn reduce_attempt(attempt: &AttemptPayload, tolerance_cents: f32) -> (f32, f32) {
    let mut voiced_count: u32 = 0;
    let mut on_pitch: u32 = 0;
    let mut sum: f32 = 0.0;
    let n = attempt
        .cents_error_frames
        .len()
        .min(attempt.voiced_frames.len());
    for i in 0..n {
        if !attempt.voiced_frames[i] {
            continue;
        }
        let c = attempt.cents_error_frames[i];
        if !c.is_finite() {
            continue;
        }
        voiced_count += 1;
        sum += c.abs();
        if c.abs() <= tolerance_cents {
            on_pitch += 1;
        }
    }
    if voiced_count == 0 {
        return (f32::NAN, 0.0);
    }
    (
        sum / voiced_count as f32,
        on_pitch as f32 / voiced_count as f32,
    )
}

/// In-window tolerance applied by the scorer when no per-spec value
/// is supplied. Matches
/// [`crate::pipeline::target_match::DEFAULT_IN_WINDOW_CENTS`].
const SCORE_TOLERANCE_CENTS: f32 = 25.0;

/// Score an attempt against the session's expected response and
/// persist a `drill_attempts` row.
///
/// `session_id` is informational — the headless twin is stateless on
/// session ids; the Tauri shell's `AppState` LRU cache is the only
/// consumer of the value. The parameter stays in the signature so the
/// in-flight resume path can be wired in without an API break.
///
/// # Errors
///
/// Returns [`DrillError::InvalidSpec`] when the spec is empty or
/// out-of-range; [`DrillError::PayloadCodec`] when postcard fails to
/// encode the spec snapshot; [`DrillError::Library`] when the SQLite
/// insert fails.
pub fn submit_drill_attempt_blocking(
    library: &RecordingsLibrary,
    session_id: DrillSessionId,
    spec: &IpcDrillSpec,
    user_pitch_midi: Option<i32>,
    attempt: &AttemptPayload,
    paired_recording_id: Option<RecordingId>,
) -> Result<IpcDrillResult, DrillError> {
    let _ = session_id; // session id not consumed by the headless twin
    validate_spec(spec)?;

    let (mean_cents_error, time_on_pitch_ratio) = reduce_attempt(attempt, SCORE_TOLERANCE_CENTS);

    // "Correct" is a coarse pass/fail: the user's median cents-error
    // is within the in-window tolerance AND (when supplied) the
    // user_pitch_midi matches the spec's first expected response.
    let cents_ok = mean_cents_error.is_finite() && mean_cents_error.abs() <= SCORE_TOLERANCE_CENTS;
    let pitch_ok = match user_pitch_midi {
        Some(m) => m == spec.expected_response_midi[0],
        None => true,
    };
    let correct = cents_ok && pitch_ok;

    // Postcard-encode the spec snapshot for the `drill_payload` blob.
    let payload =
        postcard::to_allocvec(spec).map_err(|e| DrillError::PayloadCodec(format!("{e}")))?;

    let kind_str = spec.kind.as_str();
    let row = NewDrillAttempt {
        drill_kind: kind_str,
        drill_payload: &payload,
        correct,
        mean_cents_error: f64::from(mean_cents_error),
        time_on_pitch_ratio: f64::from(time_on_pitch_ratio),
        started_at_unix_ms: attempt.started_at_unix_ms,
        finished_at_unix_ms: attempt.finished_at_unix_ms,
        recording_id: paired_recording_id,
    };
    library.insert_drill_attempt(&row)?;

    // Per-kind percentile: lower mean_cents_error = better, so the
    // returned ratio is the fraction of historical attempts whose
    // mean_cents_error is strictly higher than this attempt's.
    let percentile_in_kind = if mean_cents_error.is_finite() {
        let total = library.count_drill_attempts(kind_str)?;
        if total <= 1 {
            1.0
        } else {
            let below =
                library.count_drill_attempts_below(kind_str, f64::from(mean_cents_error))?;
            // `total - below - 1` excludes the just-inserted row from
            // the numerator so a single-row history is `1.0` and the
            // user's first attempt does not score 0/1.
            let beat = (total - below - 1).max(0);
            (beat as f32) / ((total - 1) as f32)
        }
    } else {
        0.0
    };

    Ok(IpcDrillResult {
        correct,
        mean_cents_error,
        time_on_pitch_ratio,
        percentile_in_kind,
    })
}

/// Page over the persisted drill history. The supplied `limit` is
/// clamped to [`HISTORY_LIMIT_CAP`] before the SELECT so a buggy
/// front-end cannot starve the IPC thread on a giant page.
///
/// # Errors
///
/// Returns [`DrillError::Library`] when the SQLite query fails.
pub fn list_drill_history_blocking(
    library: &RecordingsLibrary,
    filter: &HistoryFilter,
) -> Result<Vec<DrillAttempt>, DrillError> {
    let limit = filter.limit.min(HISTORY_LIMIT_CAP);
    let rows = library.list_drill_attempts(
        filter.kind.as_deref(),
        filter.since_unix_ms,
        limit,
        filter.offset,
    )?;
    let out: Vec<DrillAttempt> = rows
        .into_iter()
        .map(|r| DrillAttempt {
            id: uuid::Uuid::from_bytes(r.id).to_string(),
            drill_kind: r.drill_kind,
            correct: r.correct,
            mean_cents_error: r.mean_cents_error as f32,
            time_on_pitch_ratio: r.time_on_pitch_ratio as f32,
            started_at_unix_ms: r.started_at_unix_ms,
            finished_at_unix_ms: r.finished_at_unix_ms,
            recording_id: r.recording_id.map(|id| id.to_string()),
        })
        .collect();
    Ok(out)
}

/// Render the audio for a single prompt note. Pure-Rust headless twin
/// the Tauri `synthesize_prompt` command wraps in `spawn_blocking`.
///
/// # Errors
///
/// Returns [`DrillError::InvalidSpec`] when `duration_ms` exceeds
/// [`crate::poly::synth::MAX_PROMPT_DURATION_MS`];
/// [`DrillError::Library`] when the synth fails to render.
pub fn synthesize_prompt_blocking(note: NoteSpec, duration_ms: u32) -> Result<Vec<u8>, DrillError> {
    use crate::poly::synth::PromptSynth;
    if duration_ms > crate::poly::synth::MAX_PROMPT_DURATION_MS {
        return Err(DrillError::InvalidSpec(format!(
            "duration {duration_ms} ms exceeds {} ms cap",
            crate::poly::synth::MAX_PROMPT_DURATION_MS
        )));
    }
    let mut synth = PromptSynth::new();
    synth
        .render_wav_at_a4(note.midi, duration_ms, note.a4_hz)
        .map_err(|e| DrillError::Library(format!("{e:#}")))
}
