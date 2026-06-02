// SPDX rationale: do not modify this file. All logic is in lib.rs so iOS/Android can link the library.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
fn main() { neural_pitch_lib::run(); }
