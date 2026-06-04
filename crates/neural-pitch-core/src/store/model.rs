//! Plain-data types exchanged with `RecordingsLibrary`.
//!
//! `RecordingId` is a UUIDv7 wrapper (16 bytes) with `Display`/`FromStr` so it
//! crosses the IPC boundary as a stable hex string. `NewRecording` mirrors the
//! `recordings` columns minus the surrogate `id` and the `deleted_at_unix_ms`
//! tombstone (set by `soft_delete`). `Recording` is the read-side row shape.

use std::fmt;
use std::str::FromStr;

/// Stable handle to a row in the `recordings` table.
///
/// Backed by a UUIDv7 (16 bytes). The wrapper struct exists so the type system
/// can distinguish recording IDs from arbitrary `[u8; 16]` blobs at the IPC
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct RecordingId(pub [u8; 16]);

impl RecordingId {
    /// Mint a fresh UUIDv7-backed id. Monotonic-by-time within a millisecond
    /// resolution so default `created_at_unix_ms DESC` ordering correlates
    /// with insertion order.
    pub fn new_v7() -> Self {
        Self(*uuid::Uuid::now_v7().as_bytes())
    }

    /// Borrow the raw 16-byte UUID payload.
    pub fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

impl fmt::Display for RecordingId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Render as a canonical hyphenated UUID string for the IPC boundary.
        write!(f, "{}", uuid::Uuid::from_bytes(self.0))
    }
}

impl FromStr for RecordingId {
    type Err = uuid::Error;

    /// Parses any `uuid::Uuid::parse_str`-accepted form (hyphenated,
    /// simple, URN, or braced). `Display` always renders the canonical
    /// hyphenated form, so a round-trip through `Display` then `FromStr`
    /// is stable, but `FromStr` will also accept a non-canonical string
    /// emitted by another producer (e.g. a JS `Uuid.toString({ format:
    /// 'simple' })`).
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(*uuid::Uuid::parse_str(s)?.as_bytes()))
    }
}

/// Input record passed to `RecordingsLibrary::insert_recording`.
#[derive(Debug, Clone)]
pub struct NewRecording {
    /// Filename of the on-disk audio asset (e.g. `2026-06-04_120300_a1b2c3d4.flac`).
    pub filename: String,
    /// Wall-clock creation time in milliseconds since the Unix epoch.
    pub created_at_unix_ms: i64,
    /// Total duration of the recording in milliseconds.
    pub duration_ms: i64,
    /// Sample rate of the captured audio in Hz.
    pub sample_rate_hz: i64,
    /// Channel count of the captured audio (1 = mono, 2 = stereo).
    pub channels: i64,
    /// Bit depth of the captured PCM samples.
    pub bit_depth: i64,
    /// Container/codec format string. `"flac"` for Phase 2.0.
    pub format: String,
    /// Reference pitch for A4 in Hz (typically `440.0`).
    pub a4_hz: f64,
    /// Instrument profile slug (e.g. `"voice"`, `"violin"`).
    pub instrument_profile: String,
    /// Optional human-supplied label.
    pub user_label: Option<String>,
}

/// Output record returned by `RecordingsLibrary::list_recordings`.
#[derive(Debug, Clone)]
pub struct Recording {
    /// Surrogate primary key (UUIDv7).
    pub id: RecordingId,
    /// Filename of the on-disk audio asset.
    pub filename: String,
    /// Wall-clock creation time in milliseconds since the Unix epoch.
    pub created_at_unix_ms: i64,
    /// Total duration of the recording in milliseconds.
    pub duration_ms: i64,
    /// Sample rate of the captured audio in Hz.
    pub sample_rate_hz: i64,
    /// Channel count of the captured audio.
    pub channels: i64,
    /// Bit depth of the captured PCM samples.
    pub bit_depth: i64,
    /// Container/codec format string.
    pub format: String,
    /// Reference pitch for A4 in Hz.
    pub a4_hz: f64,
    /// Instrument profile slug.
    pub instrument_profile: String,
    /// Optional human-supplied label.
    pub user_label: Option<String>,
    /// Soft-delete tombstone in milliseconds since the Unix epoch (`Some` if deleted).
    pub deleted_at_unix_ms: Option<i64>,
}

/// Filter knob for `list_recordings`.
///
/// `#[non_exhaustive]` so adding a new variant (e.g. `DeletedOnly`,
/// `ByInstrument`) is not a breaking change for downstream `match` arms.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ListFilter {
    /// Hide rows where `deleted_at_unix_ms IS NOT NULL`.
    ActiveOnly,
    /// Return every row, including soft-deleted ones.
    IncludingDeleted,
}
