//! Desktop entry-point shim. All logic lives in `neural_pitch_lib::run`
//! so iOS and Android targets can link the library directly.
//!
//! Do not extend this file. Add behaviour to `crates/neural-pitch-core` or
//! to `src-tauri/src/lib.rs` (which is the mobile-shared entry point).
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    neural_pitch_lib::run();
}
