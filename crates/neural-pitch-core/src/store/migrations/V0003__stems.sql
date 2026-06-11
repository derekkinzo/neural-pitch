-- Stem-separation subsystem schema.
--
-- Append-only: this file is V0003 and never edits V0001/V0002 (refinery
-- refuses modified files). The new `stem_results` table is
-- non-destructive against databases at V0001/V0002 — the migration only
-- CREATEs and ADDs; nothing in `recordings`, `analysis_cache`, or
-- `drill_attempts` is removed.
--
-- The `stem_results` row is the SQL pointer; the actual FLAC bytes for
-- every stem live on disk under
-- `$APPDATA/recordings/<recording_id>/stems/{vocals,drums,bass,other}.flac`.
-- Putting audio on disk and the row pointer in SQLite mirrors how the
-- existing `recordings.filename` column works — the DB stays small,
-- the WAL stays fast, and FLAC bytes can be served directly through
-- the existing asset-protocol scope.
--
-- `separator_version` is a build-time constant
-- (`HTDEMUCS_SEPARATOR_VERSION`) baked next to the ONNX checksum so the
-- cache key survives a model swap and so concurrent installs of two app
-- versions can coexist on the same library.

CREATE TABLE stem_results (
  recording_id          BLOB    NOT NULL REFERENCES recordings(id) ON DELETE CASCADE,
  separator_version     TEXT    NOT NULL,                   -- "htdemucs-4.0.1"
  completed_at_unix_ms  INTEGER NOT NULL,
  vocals_path           TEXT    NOT NULL,
  drums_path            TEXT    NOT NULL,
  bass_path             TEXT    NOT NULL,
  other_path            TEXT    NOT NULL,
  PRIMARY KEY (recording_id, separator_version)
) STRICT;

CREATE INDEX idx_stem_results_lookup
  ON stem_results(recording_id, separator_version);

-- Additive column for the existing analysis_cache keying. Defaults to
-- NULL so every pre-V0003 row keeps its existing
-- `(recording_id, analyzer_name, analyzer_version)` cache key intact —
-- NULL reads as "the mixed full recording", and only newly written
-- stem-keyed rows carry a non-null discriminant. The PRIMARY KEY on
-- analysis_cache is left as the existing unique three-tuple; the new
-- logical key (recording_id, analyzer_name, analyzer_version, stem_kind)
-- is enforced by the application-level lookup helper. A future V0004
-- can rebuild the PK if collision-on-NULL ever surfaces in fuzzing.
ALTER TABLE analysis_cache ADD COLUMN stem_kind TEXT NULL;

UPDATE schema_version SET version = 3 WHERE id = 1;
