-- Ear-training drill subsystem schema.
--
-- Append-only: this file is V0002 and never edits V0001 (refinery refuses
-- modified files). The new `drill_attempts` table is non-destructive
-- against databases at V0001 — the migration only CREATEs; nothing in
-- `recordings` or `analysis_cache` is touched.
--
-- The `recording_id` foreign key is NULLable so practice sessions that do
-- not stash an audio recording (the common case for live mic-in drills)
-- still produce a row. ON DELETE SET NULL keeps history rows alive when
-- the underlying recording is hard-purged so the user's drill stats do
-- not vanish on cleanup.
--
-- `drill_payload` carries a postcard-encoded snapshot of the IPC drill
-- spec so a future "repeat-drill" affordance can reconstruct the prompt
-- without reading the recordings table. The read-side `DrillAttempt`
-- IPC type does not surface this column today; the column is reserved
-- for the resume path and is exercised by the insert helper.

CREATE TABLE drill_attempts (
  id                      BLOB    PRIMARY KEY,        -- UUIDv7, 16 bytes
  drill_kind              TEXT    NOT NULL,           -- 'interval' | 'sight_sing' | 'range' | …
  drill_payload           BLOB    NOT NULL,           -- postcard-encoded IpcDrillSpec snapshot
  correct                 INTEGER NOT NULL,           -- 0 / 1
  mean_cents_error        REAL    NOT NULL,
  time_on_pitch_ratio     REAL    NOT NULL,           -- [0.0, 1.0]
  started_at_unix_ms      INTEGER NOT NULL,
  finished_at_unix_ms     INTEGER NOT NULL,
  recording_id            BLOB    NULL REFERENCES recordings(id) ON DELETE SET NULL
) STRICT;

CREATE INDEX idx_drill_attempts_history
  ON drill_attempts(drill_kind, finished_at_unix_ms DESC);

UPDATE schema_version SET version = 2 WHERE id = 1;
