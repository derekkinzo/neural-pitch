# ADR-0021: Phase 2.3 cache-version bump (`pyin` 0.1 → 0.2) for range + vibrato fields

## Status

Accepted — 2026-06-05.

## Context

Phase 2.3 lifts the new `RangeReport` and `VibratoReport` analyses onto the IPC surface and into the persistent `analysis_cache` blob. The wire shape returned by `analyze_recording` (the `AnalysisSummary` struct) gains two new optional fields:

```rust
pub range:   Option<RangeReport>,
pub vibrato: Option<VibratoReport>,
```

Both fields are derived from the cached postcard `ContourResult` via the pure functions `analysis::range::compute_range` and `analysis::vibrato::compute_vibrato`. They are projected onto the wire summary by `summarize_cached`, so the cache-hit and fresh-run paths return structurally identical shapes (Phase 2.1 invariant restated).

The new convenience accessors `get_range_report` / `get_vibrato_report` re-use the same `(recording_id, analyzer_name, analyzer_version)` cache row — there is no separate row, no second cache key. The blocking projection helpers (`get_range_report_blocking` / `get_vibrato_report_blocking`) live alongside `get_contour_blocking` in `store::analysis_runtime`.

The `pyin` analyzer's `ContourResult` postcard layout has not changed in this phase, but we still need to invalidate every cached blob produced before Phase 2.3 so re-opening a pre-bump recording goes through the fresh-analysis path that populates `summarize_cached` with the new reports. `PYIN_ANALYZER_VERSION` is the cache-key fragment that buys us this invalidation: bumping it from `"0.1"` to `"0.2"` causes every previously-cached row to miss on next access and be recomputed.

## Decision

- `crates/neural-pitch-core/src/analysis/contour.rs::PYIN_ANALYZER_VERSION` moves from `"0.1"` to `"0.2"`. The IPC-side mirror in `src-tauri/src/commands.rs::DEFAULT_ANALYZER_VERSION` re-exports the constant so the two cannot drift.
- **No migration runs.** 0.1 rows stay in the `analysis_cache` table verbatim (the cache layer compares `analyzer_version` as plain SQL text — Phase 2.1 invariant). New analyses write under `(recording_id, "pyin", "0.2")` alongside any existing 0.1 row.
- `get_contour(.., "0.1")` continues to return the legacy blob for any recording analysed pre-bump. When the legacy bytes no longer round-trip through the live `ContourResult` postcard schema, `get_contour_blocking` returns a structurally-empty placeholder (`Some(_)` with empty per-frame vectors) rather than `Err(CacheCorrupted)`. Front-end consumers detect the placeholder by `f0_hz.is_empty()` and trigger a `force_refresh` analyze.
- `get_range_report` / `get_vibrato_report` apply the same back-compat: a legacy 0.1 row that cannot decode under the live schema surfaces the corresponding "insufficient data" sentinel (`RangeReport::insufficient()` / an empty `VibratoReport`). Versions other than `"0.1"` keep the strict `CacheCorrupted` semantics so a real schema regression remains loud.
- No janitor sweeps the 0.1 rows. If on-disk cache size becomes a concern, a future opportunistic cleanup can run on the first 0.2 read for the same recording.

## Consequences

- **Existing recordings** trip a one-time recompute on next access. Cached re-opens stay under the Phase-2 100 ms budget once the 0.2 row lands.
- **Front-end** sees the new `range` / `vibrato` fields populated on every fresh / cache-hit summary at `analyzer_version = "0.2"`. The TS adapter (`src/types/analysis.ts`) is extended to surface them.
- **Back-compat** for Phase 2.1 / 2.2 callers that pinned `"0.1"` is preserved through the placeholder path. They get a deterministic empty contour rather than a hard error.
- **Disk usage** grows by one row per recording analysed both pre- and post-bump until the opportunistic cleanup lands.
- **Live-tuner UX is unaffected** — the live path does not consume `analysis_cache`.

## Alternatives Considered

- **Run a destructive migration that rewrites 0.1 blobs as 0.2.** Rejected — ContributorInvariant on `PYIN_ANALYZER_VERSION` (see `analysis/contour.rs`) is "bump in lock-step with field-set or wire-ordering changes". The cache-key invalidation is the explicit, auditable mechanism for that bump; a side-channel migration that re-decodes legacy bytes against the new shape risks silently surfacing wrong values. Re-running PYIN on demand is cheap (≤ 2 s for a typical voice take on a modern CPU); a destructive migration buys us nothing.
- **Add a separate cache row keyed `(recording_id, "pyin-range", _)` / `(recording_id, "pyin-vibrato", _)`.** Rejected — both reports are pure projections of the same contour; storing them as independent rows introduces a consistency window where range and vibrato can drift relative to the contour they were derived from. The "single row per `(recording_id, analyzer_name, analyzer_version)`" invariant from ADR-0012 stays intact.
- **Hard-fail on legacy 0.1 decode.** Rejected — the spec promises `get_contour(.., "0.1")` continues to return the legacy blob. A hard error here would force every front-end consumer to special-case the version check; the placeholder + structural emptiness signal achieves the same intent at the API layer.

## References

- `crates/neural-pitch-core/src/analysis/contour.rs` — `PYIN_ANALYZER_VERSION` constant and contributor invariant.
- `crates/neural-pitch-core/src/store/analysis_runtime.rs` — `get_contour_blocking`, `get_range_report_blocking`, `get_vibrato_report_blocking`, `summarize_cached`.
- `src-tauri/src/commands.rs` — `analyze_recording`, `get_range_report`, `get_vibrato_report`, `DEFAULT_ANALYZER_VERSION`.
- ADR-0012 — Recordings library and analysis cache.
- ADR-0005 — A4 reference (configurable; no module-level state).
