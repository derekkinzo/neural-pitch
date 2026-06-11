//! Capability JSON wiring test.
//!
//! Tauri's `tauri::generate_context!` macro provides the compile-time
//! validation: an unknown permission identifier or malformed JSON would
//! fail the build before this test runs. This test pins the runtime
//! shape so a future refactor cannot silently strip the asset-protocol
//! permissions or the `main` window target. Recording-file access is
//! gated by the `assetProtocol.scope` glob in `tauri.conf.json`
//! (`$APPDATA/recordings/**`), not by an `fs:` permission — the
//! webview-side `convertFileSrc()` call is what consumes that scope.

#![allow(missing_docs)]
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

#[test]
fn default_capability_parses_and_targets_main_window() {
    let raw = include_str!("../capabilities/default.json");
    let v: serde_json::Value = serde_json::from_str(raw).expect("valid JSON");
    assert_eq!(v["identifier"], "default");
    assert_eq!(v["windows"][0], "main");
    let perms = v["permissions"].as_array().expect("permissions array");
    assert!(perms.iter().any(|p| p == "core:default"));
    assert!(perms.iter().any(|p| p == "log:default"));
    assert!(perms.iter().any(|p| p == "store:default"));
}

#[test]
fn asset_protocol_scope_pins_recordings_subtree() {
    // The webview's `convertFileSrc()` call resolves under the asset
    // protocol scope declared in `tauri.conf.json`. We pin the glob
    // here so a future refactor cannot silently widen the scope to
    // `$HOME` or `**` at filesystem root.
    let raw = include_str!("../tauri.conf.json");
    let v: serde_json::Value = serde_json::from_str(raw).expect("valid JSON");
    let scope = v["app"]["security"]["assetProtocol"]["scope"]
        .as_array()
        .expect("assetProtocol.scope array");
    let entries: Vec<&str> = scope.iter().filter_map(|e| e.as_str()).collect();
    // Pin BOTH expected scopes — APPDATA is the Windows-style path the
    // Tauri stack picks up on every platform via Tauri's path resolver,
    // APPLOCALDATA is what Linux/macOS actually use under the hood.
    // Asserting both prevents a future refactor from silently dropping
    // one platform.
    assert!(entries.iter().any(|s| s.contains("$APPDATA/recordings/")));
    assert!(
        entries
            .iter()
            .any(|s| s.contains("$APPLOCALDATA/recordings/"))
    );
    assert!(!entries.iter().any(|s| *s == "**" || s.contains("$HOME")));
}
