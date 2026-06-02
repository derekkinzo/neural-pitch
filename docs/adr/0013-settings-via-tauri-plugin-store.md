# ADR-0013: Settings via tauri-plugin-store, separate from recordings DB

## Status

Accepted — 2026-06-02.

## Context

The app has two persistent stores with very different shapes:

1. **Settings** — small JSON blob (A4 reference, default backend, recording format, advanced toggles). Read on every app launch; rarely written.
2. **Recordings library + analysis cache** — relational; per-recording rows; queryable; high write volume during analysis.

Conflating them in one SQLite DB is awkward (settings as a key-value table is needlessly relational); using JSON for the recordings library is awkward (no query support).

## Decision

- **Settings** live in a JSON blob managed by `tauri-plugin-store`, written to the OS-correct config dir.
- The Rust-side `Settings` struct carries `schema_version: u32` and uses `#[serde(default)]` on every field.
- `Settings::default()` writes `schema_version: SETTINGS_SCHEMA_VERSION` (initially 1). Migrations are explicit functions (`migrate_v1_to_v2`, etc.); the loop iterates from the stored version up to the current one.
- The frontend never touches the JSON file directly. It calls Tauri commands `get_settings()` and `set_setting(key, value)`. These commands validate, apply migration if needed, and persist.
- **Recordings library + analysis cache** stay in SQLite per ADR-0012.

## Consequences

- A user reset is `rm <config-dir>/settings.json` — settings are recoverable to defaults independently of recordings.
- Adding a settings field is a struct edit + a serde default + (if behaviour-impacting) a migration arm.
- Settings can be inspected and edited by hand in a pinch; the recordings DB cannot.
- The dual-store split adds a tiny amount of conceptual complexity.

## Alternatives Considered

- **Settings in the recordings DB** — rejected because it conflates two stores with very different lifecycles.
- **Settings in a custom JSON file written by hand** — rejected because `tauri-plugin-store` already solves the cross-platform path resolution and atomic-write problem.
- **Settings in `localStorage` (browser)** — rejected because the Rust-side audio worker also needs to read settings; sharing them through the storage plugin is cleaner than going through the IPC every read.
