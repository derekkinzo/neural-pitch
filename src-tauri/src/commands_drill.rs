//! Tauri command surface for the ear-training drill subsystem.
//!
//! Each command is a thin async wrapper over the matching `*_blocking`
//! helper in [`neural_pitch_core::training`]: validate the IPC argument,
//! hop onto a `spawn_blocking` worker, flatten the typed error to
//! `Result<T, String>` at the boundary. Mirrors the analyse / contour
//! and transcribe command shapes so the IPC surface stays uniform.
//!
//! - `start_drill` validates the spec, renders the prompt audio via
//!   [`neural_pitch_core::poly::synth::PromptSynth`], and mints a fresh
//!   [`DrillSessionId`].
//! - `submit_drill_attempt` reduces the per-frame attempt to a score
//!   row and persists it via the V0002 schema.
//! - `list_drill_history` pages over the persisted rows with a
//!   server-side limit clamp.
//! - `synthesize_prompt` exposes the additive synth as its own IPC for
//!   the front-end's per-note rendering path.
//!
//! Pure-validation commands accept `State<'_, AppState>` and discard
//! with `let _ = state;` so the IPC signature is uniform across the
//! surface.

use std::sync::Arc;

use neural_pitch_core::store::RecordingId;
use neural_pitch_core::training::{
    AttemptPayload, DrillAttempt, DrillSession, DrillSessionId, HistoryFilter, IpcDrillResult,
    IpcDrillSpec, NoteSpec, list_drill_history_blocking, start_drill_blocking,
    submit_drill_attempt_blocking, synthesize_prompt_blocking,
};
use tauri::State;

use crate::state::AppState;

/// Begin a new drill session: validate `spec`, generate the prompt
/// audio via the additive synth, mint a [`DrillSessionId`], and return
/// the [`DrillSession`] handle.
///
/// # Errors
///
/// Surfaces the helper's typed errors flattened to `String`:
/// `InvalidSpec` (out-of-range MIDI, empty prompt) or `Library` (synth
/// failure).
#[tauri::command]
#[tracing::instrument(skip(state, spec))]
pub async fn start_drill(
    state: State<'_, AppState>,
    spec: IpcDrillSpec,
) -> Result<DrillSession, String> {
    let _ = state;
    tokio::task::spawn_blocking(move || start_drill_blocking(&spec))
        .await
        .map_err(|e| format!("start_drill task panicked: {e}"))?
        .map_err(|e| format!("{e:#}"))
}

/// Score the user's response and persist a `drill_attempts` row.
///
/// # Errors
///
/// Surfaces the helper's typed errors flattened to `String`:
/// `InvalidSpec`, `PayloadCodec`, or `Library`.
#[tauri::command]
#[tracing::instrument(skip(state, attempt, spec))]
pub async fn submit_drill_attempt(
    state: State<'_, AppState>,
    session_id: String,
    spec: IpcDrillSpec,
    user_pitch_midi: Option<i32>,
    attempt: AttemptPayload,
    paired_recording_id: Option<String>,
) -> Result<IpcDrillResult, String> {
    let parsed_session: DrillSessionId = session_id
        .parse()
        .map_err(|e| format!("invalid session id: {e}"))?;
    let parsed_recording = match paired_recording_id {
        Some(s) => Some(
            s.parse::<RecordingId>()
                .map_err(|e| format!("invalid recording id: {e}"))?,
        ),
        None => None,
    };
    let library = Arc::clone(&state.library);
    tokio::task::spawn_blocking(move || {
        submit_drill_attempt_blocking(
            &library,
            parsed_session,
            &spec,
            user_pitch_midi,
            &attempt,
            parsed_recording,
        )
    })
    .await
    .map_err(|e| format!("submit_drill_attempt task panicked: {e}"))?
    .map_err(|e| format!("{e:#}"))
}

/// Page over the persisted drill history. Server-side clamps the
/// caller's `limit` to `HISTORY_LIMIT_CAP`.
///
/// # Errors
///
/// Surfaces `Library` errors flattened to `String`.
#[tauri::command]
#[tracing::instrument(skip(state, filter))]
pub async fn list_drill_history(
    state: State<'_, AppState>,
    filter: HistoryFilter,
) -> Result<Vec<DrillAttempt>, String> {
    let library = Arc::clone(&state.library);
    tokio::task::spawn_blocking(move || list_drill_history_blocking(&library, &filter))
        .await
        .map_err(|e| format!("list_drill_history task panicked: {e}"))?
        .map_err(|e| format!("{e:#}"))
}

/// Render the audio for one prompt note via the additive synth.
///
/// # Errors
///
/// Surfaces `InvalidSpec` (over-long duration) or `Library` (synth
/// failure) flattened to `String`.
#[tauri::command]
#[tracing::instrument(skip(state, note))]
pub async fn synthesize_prompt(
    state: State<'_, AppState>,
    note: NoteSpec,
    duration_ms: u32,
) -> Result<Vec<u8>, String> {
    let _ = state;
    tokio::task::spawn_blocking(move || synthesize_prompt_blocking(note, duration_ms))
        .await
        .map_err(|e| format!("synthesize_prompt task panicked: {e}"))?
        .map_err(|e| format!("{e:#}"))
}
