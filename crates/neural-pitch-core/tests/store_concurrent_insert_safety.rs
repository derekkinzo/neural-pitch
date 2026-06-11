//! Tier-1 persistence test: concurrent inserts via shared `Arc` are safe.
//!
//! 4 `std::thread::spawn` × 50 inserts each via cloned `Arc<…>`, join
//! all, assert `list_recordings(Active).len() == 200` with no panic and
//! no `SQLITE_BUSY`. The single connection sits behind a `Mutex` inside
//! the library, and WAL mode keeps readers off the writer's critical
//! section.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    clippy::cast_possible_wrap
)]

use std::path::Path;
use std::sync::Arc;
use std::thread;

use neural_pitch_core::store::{ListFilter, NewRecording, RecordingsLibrary};

#[test]
fn store_concurrent_inserts_four_threads_fifty_each_land_two_hundred_rows() {
    const THREADS: usize = 4;
    const PER_THREAD: usize = 50;
    const EXPECTED: usize = THREADS * PER_THREAD;

    let lib = Arc::new(
        RecordingsLibrary::new(Path::new(":memory:"))
            .expect("opening :memory: library should succeed once persistence ships"),
    );

    let mut handles = Vec::with_capacity(THREADS);
    for t in 0..THREADS {
        let lib_clone = Arc::clone(&lib);
        handles.push(thread::spawn(move || {
            for i in 0..PER_THREAD {
                let new = NewRecording {
                    filename: format!("t{t}_i{i}.flac"),
                    created_at_unix_ms: 1_717_502_580_000 + (t * PER_THREAD + i) as i64,
                    duration_ms: 1_000,
                    sample_rate_hz: 48_000,
                    channels: 1,
                    bit_depth: 24,
                    format: "flac".to_string(),
                    a4_hz: 440.0,
                    instrument_profile: "voice".to_string(),
                    user_label: None,
                };
                lib_clone
                    .insert_recording(new)
                    .expect("concurrent insert_recording must not return SQLITE_BUSY or error");
            }
        }));
    }

    for h in handles {
        h.join().expect("worker thread must not panic");
    }

    let rows = lib
        .list_recordings(ListFilter::ActiveOnly)
        .expect("list_recordings should succeed once persistence ships");
    assert_eq!(
        rows.len(),
        EXPECTED,
        "all {EXPECTED} concurrent inserts must land; got {}",
        rows.len()
    );
}
