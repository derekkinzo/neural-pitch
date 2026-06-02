# ADR-0012: Recordings library + per-recording analysis cache in SQLite

## Status

Accepted — 2026-06-02.

## Context

Phase 2 adds recording, playback, and offline analysis (vocal range, vibrato, eventually pYIN/PESTO transcription). Each of these produces a structured analysis output that the user wants to see instantly when they re-open a recording, without re-running the analyzer.

A simple file-per-recording-plus-sidecar-JSON layout is appealing for portability but is awkward for queries like "show all recordings from the last week" or "show recordings where the analyzer version changed". A relational store handles those queries naturally.

## Decision

- Recording metadata and per-recording analyzer outputs live in a single SQLite database via `rusqlite`.
- The DB is opened with `bundled` SQLite to dodge per-platform system-library version skew.
- The library directory is platform-conventional, resolved via the `directories` crate.
- Audio files are filesystem-resident with names `YYYY-MM-DD_HHMMSS.flac`; the SQLite `recordings` table carries a `UUID` primary key (UUIDv7 for time-ordered keys) and a `filename` column.
- The `analysis_cache` table is keyed `(recording_id, analyzer_name, analyzer_version)`; payload is opaque JSON. Re-running an analyzer at a higher version writes a new row.
- Schema migrations run via `refinery` from `src-tauri/migrations/V0001__init.sql`. The first migration also seeds the `schema_version` row.
- `PRAGMA journal_mode = WAL` is database-level (set in V0001); `PRAGMA foreign_keys = ON` and `PRAGMA synchronous = NORMAL` are connection-level (set in `open_library()` on every new connection).

## Consequences

- Re-opening a recording is instant: the analyser bound to the current default `(name, version)` either has a cache row or runs once and stores one.
- `cargo test` is fast because the test setup creates a temp DB and runs the same V0001 migration.
- Backups are simple: copy the library directory plus the SQLite file.
- Schema changes are versioned and forward-only; no destructive migrations.

## Alternatives Considered

- **`sqlx`** — rejected because async SQL adds tokio surface to the storage layer for no benefit (analysis writes are already off the audio path).
- **JSON sidecar files** — rejected for the query reasons above.
- **A separate JSON file per analyzer version** — rejected for filesystem clutter and difficulty of cleanup.
- **Diesel ORM** — rejected as heavyweight for a project of this scope.
