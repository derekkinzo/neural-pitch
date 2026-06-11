-- Recordings library + analysis-cache schema. Append-only: future
-- migrations land as new V000N files and never edit V0001 (refinery
-- refuses modified files).
--
-- WAL is set at *connection* level in `RecordingsLibrary::new` (not here) so it
-- runs even on databases that already have V0001 applied; SQLite stores
-- `journal_mode` per file/connection and the pragma is the canonical place.

CREATE TABLE schema_version (
  id          INTEGER PRIMARY KEY CHECK (id = 1),
  version     INTEGER NOT NULL
);
INSERT INTO schema_version (id, version) VALUES (1, 1);

CREATE TABLE recordings (
  id                    BLOB PRIMARY KEY,           -- UUIDv7, 16 bytes
  filename              TEXT    NOT NULL,
  created_at_unix_ms    INTEGER NOT NULL,
  duration_ms           INTEGER NOT NULL,
  sample_rate_hz        INTEGER NOT NULL,
  channels              INTEGER NOT NULL,
  bit_depth             INTEGER NOT NULL,
  format                TEXT    NOT NULL,           -- "flac" today
  a4_hz                 REAL    NOT NULL,
  instrument_profile    TEXT    NOT NULL,
  user_label            TEXT,
  deleted_at_unix_ms    INTEGER
) STRICT;

CREATE INDEX idx_recordings_created_desc
  ON recordings(created_at_unix_ms DESC);
CREATE INDEX idx_recordings_live
  ON recordings(created_at_unix_ms DESC)
  WHERE deleted_at_unix_ms IS NULL;

CREATE TABLE analysis_cache (
  recording_id           BLOB    NOT NULL REFERENCES recordings(id) ON DELETE CASCADE,
  analyzer_name          TEXT    NOT NULL,
  analyzer_version       TEXT    NOT NULL,
  computed_at_unix_ms    INTEGER NOT NULL,
  result_format_version  INTEGER NOT NULL,
  result_blob            BLOB    NOT NULL,
  PRIMARY KEY (recording_id, analyzer_name, analyzer_version)
) STRICT;
