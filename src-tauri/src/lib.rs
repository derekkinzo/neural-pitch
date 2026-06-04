//! NeuralPitch Tauri shell — all logic lives in the library so iOS / Android
//! builds can link it.
//!
//! Phase 1.2 wires the Phase 1.1 `neural-pitch-core` DSP pipeline into Tauri
//! 2 commands per ADR-0014. Streaming `PitchUpdate` frames flow over a
//! `tauri::ipc::Channel<PitchUpdate>` that the JavaScript side constructs
//! and passes into `start_capture` as a command argument; the Rust shell
//! does not own or create the channel.
#![warn(missing_docs)]

pub mod commands;
pub mod sink;
pub mod state;

use std::sync::Arc;

use neural_pitch_core::settings::{TunerSettings, migrate};
use tauri::Manager;
use tauri_plugin_store::StoreExt;
use tracing_subscriber::EnvFilter;

use crate::state::AppState;

/// Entry point invoked by both desktop main.rs and mobile platform
/// frameworks.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,neural_pitch=debug")),
        )
        .init();
    tracing::info!("NeuralPitch Phase 1.2 starting");

    // Builder failure at startup is unrecoverable — no front-end exists
    // yet, no state is persisted, the OS-level shell window is not open.
    // Allow the panic in this single bootstrap path. ADR-0014 documents
    // this exception.
    #[allow(clippy::expect_used)]
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::default().build())
        .plugin(tauri_plugin_store::Builder::default().build())
        .setup(|app| {
            let store = app.store("settings.json")?;
            let settings = load_or_init_settings(&store);
            let state = AppState::new(Arc::clone(&store), settings);
            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            commands::start_capture,
            commands::stop_capture,
            commands::configure,
            commands::get_settings,
            commands::set_setting,
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
