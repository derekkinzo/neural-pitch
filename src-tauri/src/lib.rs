//! NeuralPitch Tauri shell — all logic lives in the library so platform
//! entry points (desktop today) link it.
//!
//! Streaming `PitchUpdate` frames flow over a
//! `tauri::ipc::Channel<PitchUpdate>` that the JavaScript side constructs
//! and passes into `start_capture` as a command argument; the Rust shell
//! does not own or create the channel.
#![warn(missing_docs)]

pub mod commands;
pub mod commands_drill;
pub mod sink;
pub mod state;
// Phase 3 — file-import / Basic Pitch transcribe / MIDI export.
// Gated behind `feature = "neural"` because the GREEN path depends on
// `neural-pitch-core/poly` (which itself only compiles under
// `neural-pitch-core/neural`). Under `--no-default-features` the module
// is compiled out entirely so the classical-only build stays clean.
#[cfg(feature = "neural")]
pub mod transcribe;
// Phase 5 — HTDemucs four-stem separation surface (vocals / drums /
// bass / other). Same gating story as `transcribe`: the headless twin
// in `stems.rs` decodes recordings + writes FLACs through APIs that
// only compile under `neural`, so the whole module is compiled out
// under `--no-default-features`.
#[cfg(feature = "neural")]
pub mod stems;

use std::path::PathBuf;
use std::sync::Arc;

use neural_pitch_core::settings::{TunerSettings, migrate};
use neural_pitch_core::store::RecordingsLibrary;
use tauri::Manager;
use tauri_plugin_store::StoreExt;
use tracing_subscriber::EnvFilter;

use crate::state::AppState;

/// Entry point invoked by the desktop main.rs.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,neural_pitch=debug")),
        )
        .init();
    tracing::info!(version = env!("CARGO_PKG_VERSION"), "NeuralPitch starting");

    // Builder failure at startup is unrecoverable — no front-end exists,
    // no state is persisted, the OS-level shell window is not open. The
    // single bootstrap path is the documented exception to the
    // `expect_used` lint.
    #[allow(clippy::expect_used)]
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::default().build())
        .plugin(tauri_plugin_store::Builder::default().build())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            let store = app.store("settings.json")?;
            let settings = load_or_init_settings(&store);
            // Resolve the per-platform app-data dir for the recordings
            // SQLite + FLAC files. Tauri's `path::app_data_dir` is the
            // canonical location (Library/Application Support on macOS,
            // %APPDATA% on Windows, ~/.local/share on Linux). On
            // resolution failure we fall back to the current working
            // directory so the app at least starts; warn so operators
            // can fix the deployment.
            let app_data = app.path().app_data_dir().unwrap_or_else(|e| {
                tracing::warn!(
                    error = %e,
                    "could not resolve app_data_dir; falling back to cwd"
                );
                PathBuf::from(".")
            });
            let recordings_dir = app_data.join("recordings");
            // Best-effort: ensure the recordings dir exists. The library
            // open below also creates the parent directory if needed.
            if let Err(e) = std::fs::create_dir_all(&recordings_dir) {
                tracing::warn!(
                    error = %e,
                    path = %recordings_dir.display(),
                    "could not create recordings directory at startup",
                );
            }
            let db_path = recordings_dir.join("library.sqlite");
            let library = Arc::new(
                RecordingsLibrary::new(&db_path)
                    .map_err(|e| format!("open recordings library: {e:#}"))?,
            );
            let state = AppState::new(Arc::clone(&store), settings, library, recordings_dir);
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::start_capture,
            commands::stop_capture,
            commands::configure,
            commands::get_settings,
            commands::set_setting,
            commands::start_recording,
            commands::stop_recording,
            commands::get_recording_path,
            commands::list_recordings,
            commands::delete_recording,
            commands::rename_recording,
            commands::analyze_recording,
            commands::analyze_recording_with_backend,
            commands::get_contour,
            commands::list_analyses,
            commands::delete_analysis,
            commands::get_range_report,
            commands::get_vibrato_report,
            commands::cancel_analysis,
            commands::get_capabilities,
            commands::get_model_status,
            #[cfg(feature = "neural")]
            commands::import_audio_file,
            #[cfg(feature = "neural")]
            commands::transcribe_recording,
            #[cfg(feature = "neural")]
            commands::export_midi,
            #[cfg(feature = "neural")]
            commands::separate_stems,
            #[cfg(feature = "neural")]
            commands::read_stem_audio,
            #[cfg(feature = "neural")]
            commands::cancel_stem_separation,
            #[cfg(feature = "neural")]
            commands::download_stem_model,
            #[cfg(feature = "neural")]
            commands::export_stem,
            commands_drill::start_drill,
            commands_drill::submit_drill_attempt,
            commands_drill::list_drill_history,
            commands_drill::synthesize_prompt,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Load persisted settings from the store, applying migrations. Returns
/// `TunerSettings::default()` on any deserialisation failure so a hand-
/// edited or otherwise corrupt blob never blocks app startup.
fn load_or_init_settings(store: &tauri_plugin_store::Store<tauri::Wry>) -> TunerSettings {
    if let Some(raw) = store.get(commands::SETTINGS_STORE_KEY) {
        let migrated = migrate(raw);
        match serde_json::from_value::<TunerSettings>(migrated) {
            Ok(s) => match s.validate() {
                Ok(()) => return s,
                Err(e) => {
                    tracing::warn!(error = %e, "persisted settings failed validation; using defaults");
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "persisted settings failed to deserialise; using defaults");
            }
        }
    }
    let defaults = TunerSettings::default();
    let value = match serde_json::to_value(&defaults) {
        Ok(v) => v,
        Err(e) => {
            tracing::warn!(error = %e, "failed to seed default settings into store");
            return defaults;
        }
    };
    store.set(commands::SETTINGS_STORE_KEY, value);
    if let Err(e) = store.save() {
        tracing::warn!(error = %e, "failed to save default settings to store");
    }
    defaults
}
