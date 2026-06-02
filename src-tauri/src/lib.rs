//! NeuralPitch Tauri shell — all logic lives in the library so iOS / Android builds can link it.
#![warn(missing_docs)]

use tracing_subscriber::EnvFilter;

/// Entry point invoked by both desktop main.rs and mobile platform frameworks.
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,neural_pitch=debug")),
        )
        .init();
    tracing::info!("NeuralPitch Phase 0 skeleton starting");

    #[allow(clippy::expect_used)] // Tauri builder failure at startup is unrecoverable.
    tauri::Builder::default()
        .plugin(tauri_plugin_log::Builder::default().build())
        .invoke_handler(tauri::generate_handler![greet])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Phase 0 placeholder command. Replaced in Phase 1 with start_capture / stop_capture / etc.
#[tauri::command]
fn greet(name: &str) -> String {
    format!("Hello, {name}! NeuralPitch core says: A4 default = {} Hz", 440.0)
}
