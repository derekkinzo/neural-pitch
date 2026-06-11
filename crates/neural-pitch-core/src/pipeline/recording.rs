//! Audio capture / FLAC recording pipeline.
//!
//! Implements the trait surface, error type, artifact shape, and worker
//! used to encode captured audio into 24-bit / mono / 48 kHz FLAC
//! files. The audio callback never touches this module — the DSP
//! worker is wired to fan hop-aligned slices out through a bounded
//! `std::sync::mpsc::sync_channel` and the [`RecordingWorker`] drains
//! it onto a [`RecordingSink`].
//!
//! The [`RecordingSink`] / [`RecordingWorker`] core is exercised by the
//! crate's unit tests. The DSP-worker fan-out and the `start_recording`
//! / `stop_recording` Tauri commands are wired in
//! `src-tauri/src/commands.rs` using a bounded `sync_channel`. The
//! encoder thread receives hop-sized `Vec<f32>` slices via `try_send`;
//! the producer-side increments the shared `dropped_windows`
//! `AtomicU64` on `TrySendError::Full`.
//!
//! Design anchors:
//! - Recording fidelity: 48 kHz / 24-bit / mono / FLAC.
//! - rtrb is the only legal egress from the audio
//!   callback; FLAC encoding sits two thread hops away on a worker thread.
//! - `Drop` impls never panic; partial files are abandoned on
//!   drop and never exposed to the library.

#[cfg(feature = "flac")]
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

#[cfg(feature = "flac")]
use std::fs::OpenOptions;
#[cfg(feature = "flac")]
use std::io::{BufWriter, Write};

/// Sink trait for any single-shot recording target.
///
/// The trait is core-side (P2): no Tauri imports. `finalize` consumes
/// `Box<Self>` so the type system enforces single-shot finalize and frees
/// the implementation to assert non-drop-finalization invariants.
pub trait RecordingSink: Send {
    /// Append `samples` (mono `f32` in `[-1.0, 1.0]`) to the recording.
    fn write(&mut self, samples: &[f32]) -> Result<(), RecordingSinkError>;

    /// Flush, fsync, atomically rename the partial file into place, and
    /// return the resulting [`RecordingArtifact`]. Consumes the sink.
    fn finalize(self: Box<Self>) -> Result<RecordingArtifact, RecordingSinkError>;
}

/// Successful outcome of a finalized recording.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RecordingArtifact {
    /// Final on-disk path of the recording.
    pub path: PathBuf,
    /// Total duration in milliseconds, computed from
    /// `sample_count * 1000 / sample_rate_hz`.
    pub duration_ms: u64,
    /// Total number of `f32` mono samples written through the sink.
    pub sample_count: u64,
    /// Sample rate of the recording, in Hertz (locked to 48 000).
    pub sample_rate_hz: u32,
}

/// Errors raised by [`RecordingSink::write`] and [`RecordingSink::finalize`].
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RecordingSinkError {
    /// I/O error from the underlying writer (disk full, permission denied,
    /// etc.).
    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    /// The FLAC encoder returned a structured error.
    #[error("encoder: {0}")]
    Encoder(String),

    /// `finalize()` was called twice (or after a previous `write` aborted
    /// the sink).
    #[error("already finalized")]
    AlreadyFinalized,

    /// `create()` rejected the supplied configuration (e.g. non-48 kHz).
    #[error("invalid config: {0}")]
    InvalidConfig(String),
}

/// FLAC-backed recording sink.
///
/// Production recording sink. `write()` converts the supplied `f32`
/// slice to 24-bit signed PCM packed in `i32` and appends to an in-memory
/// buffer (`samples_i32`). On `finalize()` the partial file is truncated,
/// the FLAC stream is encoded with `flacenc-rs` and written through the
/// buffered writer, fsynced, then atomically renamed to `<path>`. On
/// `Drop` without `finalize()`, the partial is best-effort deleted.
///
/// **Memory profile:** the in-memory buffer grows linearly with recording
/// length — ~12 MB per minute at 48 kHz mono i32. For short takes (well
/// under 30 minutes) the in-memory buffer is acceptable; longer
/// recordings are out of scope for the current encoder.
/// Disk-full now surfaces only at `finalize()` time when the encoded
/// stream is written + fsynced — there is no per-write probe (the prior
/// 4-byte probe was net-zero because the partial file was rewritten in
/// truncate mode at finalize anyway, and BufWriter's 8 KiB buffer plus the
/// kernel page cache often masked ENOSPC until close on most filesystems).
#[cfg(feature = "flac")]
pub struct FlacRecordingSink {
    /// Final destination path (after rename).
    path: PathBuf,
    /// `<path>.partial` — where bytes flow until finalize.
    partial_path: PathBuf,
    /// Recording sample rate (locked to 48 000).
    sample_rate_hz: u32,
    /// Running tally of mono samples written through `write()`.
    samples_written: u64,
    /// Master sample buffer (24-bit PCM packed in `i32`). Grows on demand;
    /// in steady state `write()` only appends, so allocation is amortised
    /// to O(log N) re-grows.
    samples_i32: Vec<i32>,
    /// Set by `finalize()`; consulted by `Drop` to suppress partial deletion.
    finalized: bool,
}

#[cfg(feature = "flac")]
impl FlacRecordingSink {
    /// Open a new FLAC recording at `path`. Validates the locked recording defaults
    /// (48 kHz, mono, 24-bit) and opens `<path>.partial` for buffered
    /// writes.
    ///
    /// # Errors
    ///
    /// Returns [`RecordingSinkError::InvalidConfig`] if `sample_rate_hz` is
    /// not 48 000, or [`RecordingSinkError::Io`] if the partial file cannot
    /// be created.
    pub fn create(path: impl AsRef<Path>, sample_rate_hz: u32) -> Result<Self, RecordingSinkError> {
        if sample_rate_hz != 48_000 {
            return Err(RecordingSinkError::InvalidConfig(format!(
                "sample_rate_hz must be 48000 (got {sample_rate_hz})"
            )));
        }
        let path: PathBuf = path.as_ref().to_path_buf();
        let partial_path = partial_path_for(&path);
        // Eagerly create + truncate the partial file so create-time errors
        // (path component missing, permission denied, target dir
        // read-only) surface here rather than at finalize time. We close
        // the handle immediately; finalize re-opens with truncate to write
        // the encoded FLAC bytes. This trades one extra open() syscall for
        // a clean error surface at create time.
        let mut opts = OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            // Recordings can contain sensitive audio. Restrict to owner
            // read/write on unix so the file is not world-readable on a
            // shared multi-user install. The umask still applies to the
            // subsequent re-open in finalize, but the existing file
            // permissions persist across truncation.
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let file = opts.open(&partial_path)?;
        // Close the handle immediately — `write()` no longer probes the
        // file, so we do not need a long-lived BufWriter here.
        drop(file);
        Ok(Self {
            path,
            partial_path,
            sample_rate_hz,
            samples_written: 0,
            samples_i32: Vec::new(),
            finalized: false,
        })
    }

    /// Final destination path (after `finalize()` rename).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// `<path>.partial` — used by tests that need to assert the partial file
    /// no longer exists after Drop.
    pub fn partial_path(&self) -> &Path {
        &self.partial_path
    }

    /// Running sample count.
    pub fn sample_count(&self) -> u64 {
        self.samples_written
    }

    /// Locked recording sample rate (48 000).
    pub fn sample_rate_hz(&self) -> u32 {
        self.sample_rate_hz
    }

    /// Channel count — `1` (mono).
    pub const CHANNELS: u16 = 1;

    /// Bit depth — `24`.
    pub const BIT_DEPTH: u8 = 24;
}

/// Convert one `f32` sample in `[-1.0, 1.0]` to a 24-bit signed integer
/// left-justified in `i32`. Saturating arithmetic; never panics.
#[cfg(feature = "flac")]
#[inline]
fn f32_to_pcm24(s: f32) -> i32 {
    // 24-bit range: [-2^23, 2^23 - 1]. We multiply by 2^23 - 1 (8_388_607)
    // to keep the positive peak inside the representable range.
    let clamped = s.clamp(-1.0, 1.0);
    let scaled = (clamped * 8_388_607.0_f32).round();
    // `as i32` on f32 saturates to the i32 range, so this conversion is
    // total. The clamp above keeps us well inside [-2^23 + 1, 2^23 - 1].
    scaled as i32
}

/// Compute the partial path for a given target path by appending the
/// `.partial` suffix to the file name (not as a separate extension).
#[cfg(feature = "flac")]
fn partial_path_for(target: &Path) -> PathBuf {
    let mut s: std::ffi::OsString = target.as_os_str().to_owned();
    s.push(".partial");
    PathBuf::from(s)
}

#[cfg(feature = "flac")]
impl RecordingSink for FlacRecordingSink {
    fn write(&mut self, samples: &[f32]) -> Result<(), RecordingSinkError> {
        if self.finalized {
            return Err(RecordingSinkError::AlreadyFinalized);
        }
        // Reserve once — Vec::extend on an iterator with size_hint already
        // reserves, but being explicit makes the steady-state cost obvious
        // (amortised O(1) appends, no per-sample reallocation).
        self.samples_i32.reserve(samples.len());
        for &s in samples {
            self.samples_i32.push(f32_to_pcm24(s));
        }

        self.samples_written += samples.len() as u64;
        Ok(())
    }

    fn finalize(mut self: Box<Self>) -> Result<RecordingArtifact, RecordingSinkError> {
        if self.finalized {
            // The `Box<Self>`-consuming signature of `finalize` already
            // makes double-finalize structurally unreachable from safe
            // code; this guard is a belt-and-suspenders check for any
            // wrapper sink that does its own bookkeeping.
            return Err(RecordingSinkError::AlreadyFinalized);
        }

        // Encode the buffered samples into a FLAC byte stream.
        let bytes = encode_flac_bytes(
            &self.samples_i32,
            self.sample_rate_hz,
            usize::from(Self::BIT_DEPTH),
        )?;

        // Open the partial file in truncate mode and write the encoded
        // bytes in a buffered, fsync'd dump. ENOSPC surfaces here at the
        // very latest (BufWriter::write_all on /dev/full returns ENOSPC
        // synchronously once the kernel ring buffer is full; sync_all
        // forces the remainder out to disk and surfaces any deferred
        // ENOSPC the page cache had absorbed).
        let mut opts = OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let file = opts.open(&self.partial_path)?;
        let mut writer = BufWriter::new(file);
        writer.write_all(&bytes)?;
        writer.flush()?;
        // Drop into the underlying File so we can fsync.
        let file = writer
            .into_inner()
            .map_err(|e| RecordingSinkError::Io(e.into_error()))?;
        file.sync_all()?;
        drop(file);

        // Atomic rename — `<path>.partial` → `<path>`.
        std::fs::rename(&self.partial_path, &self.path)?;

        // Mark finalized so Drop doesn't try to clean up.
        self.finalized = true;

        let sample_count = self.samples_written;
        let duration_ms = sample_count.saturating_mul(1000) / u64::from(self.sample_rate_hz).max(1);
        Ok(RecordingArtifact {
            path: self.path.clone(),
            duration_ms,
            sample_count,
            sample_rate_hz: self.sample_rate_hz,
        })
    }
}

/// Encode buffered i32 PCM samples into a complete FLAC byte stream.
#[cfg(feature = "flac")]
fn encode_flac_bytes(
    samples: &[i32],
    sample_rate_hz: u32,
    bits_per_sample: usize,
) -> Result<Vec<u8>, RecordingSinkError> {
    use flacenc::component::BitRepr;
    use flacenc::error::Verify;

    let config = flacenc::config::Encoder::default()
        .into_verified()
        .map_err(|e| RecordingSinkError::Encoder(format!("verify config: {e:?}")))?;
    let source = flacenc::source::MemSource::from_samples(
        samples,
        1, // mono
        bits_per_sample,
        sample_rate_hz as usize,
    );
    let block_size = config.block_size;
    let stream = flacenc::encode_with_fixed_block_size(&config, source, block_size)
        .map_err(|e| RecordingSinkError::Encoder(format!("encode: {e:?}")))?;
    let mut sink = flacenc::bitsink::ByteSink::new();
    stream
        .write(&mut sink)
        .map_err(|e| RecordingSinkError::Encoder(format!("serialise: {e:?}")))?;
    Ok(sink.as_slice().to_vec())
}

#[cfg(feature = "flac")]
impl Drop for FlacRecordingSink {
    fn drop(&mut self) {
        // Drop never panics. When the sink is dropped
        // without `finalize()`, the partial FLAC file is corrupt and must
        // be best-effort deleted so the library never sees it.
        if self.finalized {
            return;
        }
        match std::fs::remove_file(&self.partial_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                tracing::warn!(
                    target: "neural_pitch::recording",
                    error = %e,
                    path = %self.partial_path.display(),
                    "failed to remove partial recording on drop",
                );
            }
        }
    }
}

/// In-memory recording sink for unit tests.
///
/// Captures every `write()` slice into an inner `Vec<f32>` and synthesises
/// a [`RecordingArtifact`] on `finalize()` with the caller-supplied path —
/// no filesystem I/O. Always exported (no `cfg(test)` gate) so integration
/// tests under `tests/` can reach it without a separate `test-utils`
/// feature flag flip; the type is small and inert in release builds.
pub struct MockRecordingSink {
    /// Path stamped onto the synthesised [`RecordingArtifact`].
    path: PathBuf,
    /// Sample rate stamped onto the synthesised [`RecordingArtifact`].
    sample_rate_hz: u32,
    /// All samples ever passed to `write()`, concatenated.
    samples: Vec<f32>,
    /// Set by `finalize()`; double-finalize returns
    /// [`RecordingSinkError::AlreadyFinalized`].
    finalized: bool,
}

impl MockRecordingSink {
    /// New mock sink that will report `path` / `sample_rate_hz` on
    /// `finalize()`.
    pub fn new(path: impl Into<PathBuf>, sample_rate_hz: u32) -> Self {
        Self {
            path: path.into(),
            sample_rate_hz,
            samples: Vec::new(),
            finalized: false,
        }
    }

    /// Read-only view of every sample ever passed to `write()`.
    pub fn samples(&self) -> &[f32] {
        &self.samples
    }
}

impl RecordingSink for MockRecordingSink {
    fn write(&mut self, samples: &[f32]) -> Result<(), RecordingSinkError> {
        if self.finalized {
            return Err(RecordingSinkError::AlreadyFinalized);
        }
        self.samples.extend_from_slice(samples);
        Ok(())
    }

    fn finalize(mut self: Box<Self>) -> Result<RecordingArtifact, RecordingSinkError> {
        if self.finalized {
            return Err(RecordingSinkError::AlreadyFinalized);
        }
        self.finalized = true;
        let sample_count = self.samples.len() as u64;
        let duration_ms = sample_count.saturating_mul(1000) / u64::from(self.sample_rate_hz).max(1);
        Ok(RecordingArtifact {
            path: self.path.clone(),
            duration_ms,
            sample_count,
            sample_rate_hz: self.sample_rate_hz,
        })
    }
}

/// Stable per-recording identifier (uuid_v7 in production, opaque string in
/// the stub). Defined here so the `RecordingHandle` API surface compiles in
/// the core crate; the shell crate threads the same type through Tauri
/// commands.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RecordingId(pub String);

impl RecordingId {
    /// Wrap an opaque string. Real impl uses uuid_v7.
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

/// Errors raised by the recording worker.
///
/// Wraps [`RecordingSinkError`] plus worker-specific failure modes
/// (cancellation, panic in the spawn-blocking task, channel disconnect).
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum RecordingError {
    /// The underlying sink reported an error mid-recording.
    #[error(transparent)]
    Sink(#[from] RecordingSinkError),

    /// The disk filled up mid-write (sentinel for the §6 disk-full path).
    #[error("disk full")]
    DiskFull,

    /// The fan-out channel disconnected before the worker saw a stop signal.
    #[error("upstream channel disconnected")]
    Disconnected,

    /// The worker task panicked or was aborted.
    #[error("worker join error: {0}")]
    Join(String),
}

/// Progress events emitted by the [`RecordingWorker`].
///
/// Wired through `tauri::ipc::Channel<RecordingProgress>` from the shell.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum RecordingProgress {
    /// Periodic ~5 Hz heartbeat.
    Tick {
        /// Total samples written so far.
        sample_count: u64,
        /// Wall-clock duration of the recording.
        duration_ms: u64,
        /// Cumulative count of windows the DSP worker tried to fan out
        /// while the recording channel was full.
        dropped_windows: u64,
    },
    /// Terminal failure event — the recording was aborted; no library row
    /// is written.
    Failed {
        /// Human-readable reason; mirrors the underlying [`RecordingError`].
        reason: String,
    },
    /// Terminal success event — sink finalized, file in place.
    Finalized {
        /// The resulting on-disk artifact.
        artifact: RecordingArtifact,
    },
}

/// Handle returned to the shell by `start_recording`. Holds the cancel
/// token, the join handle, and the dropped-windows counter so the shell
/// can surface backpressure on each [`RecordingProgress::Tick`].
pub struct RecordingHandle {
    /// Stable identifier for this recording.
    id: RecordingId,
    /// Cancellation token; `stop()` flips it.
    cancel: CancellationToken,
    /// Counter shared with the DSP worker: increments every time the
    /// fan-out channel is full and a hop slice has to be dropped.
    dropped_windows: Arc<AtomicU64>,
    /// Encoder thread join handle. `None` when the handle is constructed
    /// purely as a data carrier (e.g. in tests that drive the worker
    /// directly via `RecordingWorker::run`).
    join: Option<std::thread::JoinHandle<Result<RecordingArtifact, RecordingError>>>,
}

impl RecordingHandle {
    /// New handle without a join — for tests that drive the worker
    /// synchronously and only need the cancel + counters.
    pub fn new(
        id: RecordingId,
        cancel: CancellationToken,
        dropped_windows: Arc<AtomicU64>,
    ) -> Self {
        Self {
            id,
            cancel,
            dropped_windows,
            join: None,
        }
    }

    /// New handle wrapping an encoder-thread join handle.
    pub fn with_join(
        id: RecordingId,
        cancel: CancellationToken,
        dropped_windows: Arc<AtomicU64>,
        join: std::thread::JoinHandle<Result<RecordingArtifact, RecordingError>>,
    ) -> Self {
        Self {
            id,
            cancel,
            dropped_windows,
            join: Some(join),
        }
    }

    /// Stable identifier for this recording.
    pub fn id(&self) -> &RecordingId {
        &self.id
    }

    /// Read the cumulative dropped-window counter.
    pub fn dropped_windows(&self) -> u64 {
        self.dropped_windows
            .load(std::sync::atomic::Ordering::Relaxed)
    }

    /// Cancel the underlying token. Tests can call this to request shutdown
    /// without consuming the handle.
    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    /// Cancel the worker, await the encoder thread, return the artifact.
    ///
    /// **Unbounded wait.** This call blocks the caller until the encoder
    /// thread exits, which can be arbitrarily long if `finalize()` is
    /// stuck on a slow disk fsync. Prefer [`Self::stop_with_timeout`] in
    /// async / Tauri-command contexts; this entry point is kept for
    /// existing tests and code paths that already have their own timing
    /// envelope.
    ///
    /// # Errors
    ///
    /// Returns [`RecordingError`] if the sink reported an error before the
    /// stop signal, or if the worker thread panicked.
    pub fn stop(mut self) -> Result<RecordingArtifact, RecordingError> {
        self.cancel.cancel();
        match self.join.take() {
            Some(join) => match join.join() {
                Ok(res) => res,
                Err(_) => Err(RecordingError::Join("encoder thread panicked".into())),
            },
            None => Err(RecordingError::Join(
                "no join handle — handle was constructed without a worker".into(),
            )),
        }
    }

    /// Like [`Self::stop`] but bounds the wait on the encoder thread.
    ///
    /// Polls `JoinHandle::is_finished()` at a coarse cadence until either
    /// the join completes or `budget` elapses. On timeout returns
    /// [`RecordingError::Join`] carrying `"timeout"` and the join handle
    /// is dropped (detached). The CancellationToken has already been
    /// flipped, so the encoder thread will eventually finish on its own
    /// even after we stop waiting; this matches the
    /// `commands::stop_capture` pattern in `src-tauri/`.
    ///
    /// # Errors
    ///
    /// Returns [`RecordingError::Join`] on timeout or panic, or the
    /// underlying [`RecordingError`] from the worker on a clean exit.
    pub fn stop_with_timeout(
        mut self,
        budget: std::time::Duration,
    ) -> Result<RecordingArtifact, RecordingError> {
        self.cancel.cancel();
        let Some(join) = self.join.take() else {
            return Err(RecordingError::Join(
                "no join handle — handle was constructed without a worker".into(),
            ));
        };
        let deadline = std::time::Instant::now() + budget;
        while std::time::Instant::now() < deadline {
            if join.is_finished() {
                return match join.join() {
                    Ok(res) => res,
                    Err(_) => Err(RecordingError::Join("encoder thread panicked".into())),
                };
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        Err(RecordingError::Join("timeout".into()))
    }
}

impl Drop for RecordingHandle {
    /// `Drop` never panics. If the handle is dropped without
    /// `stop()` (e.g. the shell shuts down while a recording is live), we
    /// MUST still flip the cancellation token so the encoder thread does
    /// not spin a `recv_timeout` loop indefinitely after the producer
    /// side disconnects. We deliberately do NOT call `JoinHandle::join()`
    /// here — joining can block on a slow `finalize()` fsync and that
    /// would hang application shutdown. The thread either exits on its
    /// own (typically within a few milliseconds of the cancel) or is
    /// reaped by the OS at process exit; either is acceptable.
    fn drop(&mut self) {
        self.cancel.cancel();
        // Detach: the join handle is dropped, the underlying thread
        // remains schedulable until it observes the cancellation token.
        // No `join()` call here — see method-level comment for why.
        drop(self.join.take());
    }
}

/// Encoder-side worker that drains the fan-out channel and pushes hop
/// slices into a [`RecordingSink`].
///
/// Lives in `core` (not `src-tauri`) so the unit tests can drive the loop
/// against a [`MockRecordingSink`] without the Tauri shell. The production
/// shell wraps this in `tokio::task::spawn_blocking`; the tests use a
/// plain `std::thread::spawn`.
pub struct RecordingWorker {
    /// Sink to push hop slices into.
    sink: Box<dyn RecordingSink>,
    /// Receiver end of the fan-out channel from the DSP worker.
    rx: std::sync::mpsc::Receiver<Vec<f32>>,
    /// Cancellation token. The worker checks this between recv attempts.
    cancel: CancellationToken,
    /// Shared counter incremented by the DSP worker each time the fan-out
    /// channel is full; surfaced on each [`RecordingProgress::Tick`] and
    /// on `RecordingHandle`.
    dropped_windows: Arc<AtomicU64>,
}

impl RecordingWorker {
    /// New worker. Holds the moving parts; `run()` is the actual loop.
    pub fn new(
        sink: Box<dyn RecordingSink>,
        rx: std::sync::mpsc::Receiver<Vec<f32>>,
        cancel: CancellationToken,
        dropped_windows: Arc<AtomicU64>,
    ) -> Self {
        Self {
            sink,
            rx,
            cancel,
            dropped_windows,
        }
    }

    /// Read-only handle to the dropped-windows counter (for tests).
    pub fn dropped_windows_handle(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.dropped_windows)
    }

    /// Spawn the worker on a `std::thread`. Returns a [`RecordingHandle`]
    /// wrapping the join.
    ///
    /// # Errors
    ///
    /// Returns [`RecordingError`] if the OS rejects the thread spawn.
    pub fn spawn(self, id: RecordingId) -> Result<RecordingHandle, RecordingError> {
        let cancel = self.cancel.clone();
        let dropped = Arc::clone(&self.dropped_windows);
        let join = std::thread::Builder::new()
            .name("neural-pitch-recording".to_string())
            .spawn(move || self.run())
            .map_err(|e| RecordingError::Join(format!("spawn recording worker: {e}")))?;
        Ok(RecordingHandle::with_join(id, cancel, dropped, join))
    }

    /// Run the loop on the calling thread. Used by tests that want to
    /// drive the worker synchronously without spawning.
    ///
    /// Loop semantics — drain `rx` until either:
    /// - the cancellation token is tripped (clean stop, finalize the sink),
    /// - the channel disconnects (producer side dropped — also finalize).
    ///
    /// On any sink error mid-write, the sink is dropped (its Drop impl
    /// removes the partial file) and a [`RecordingError`] is returned.
    ///
    /// Backpressure accounting — `dropped_windows` is bumped by the
    /// **producer** side of the bounded `sync_channel`, on
    /// `TrySendError::Full`. The worker never increments it: the only
    /// honest observation point is the producer (which sees the channel
    /// reject a hop slice). The `try_recv` drain inside the loop body is
    /// purely a recv-batching optimisation — every window drained that
    /// way is processed, not dropped.
    ///
    /// # Errors
    ///
    /// Returns [`RecordingError`] if the sink errors during `write` or
    /// `finalize`.
    pub fn run(mut self) -> Result<RecordingArtifact, RecordingError> {
        // Polling shape: the worker blocks on the channel with a short
        // timeout so the cancellation token can be observed promptly. The
        // poll interval is small enough that "cancel without producing
        // anything" finishes well within the §6 50 ms budget.
        let poll = std::time::Duration::from_millis(2);
        loop {
            if self.cancel.is_cancelled() {
                break;
            }
            match self.rx.recv_timeout(poll) {
                Ok(window) => {
                    // Sink errors mid-recording propagate via `?`; the worker
                    // owns `self.sink` so the `Drop` impl runs as the function
                    // unwinds, removing the partial file. ENOSPC is mapped to
                    // the typed `DiskFull` variant for the front-end.
                    self.sink.write(&window).map_err(map_sink_err)?;
                    // Drain any extra windows that piled up behind this
                    // one. This is a recv-batching optimisation; it does
                    // NOT count as backpressure (those windows were not
                    // dropped — they are processed through the sink). The
                    // producer side accounts for genuine backpressure on
                    // `TrySendError::Full`.
                    while let Ok(extra) = self.rx.try_recv() {
                        self.sink.write(&extra).map_err(map_sink_err)?;
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }

        // Drain anything still buffered in the channel after cancel — keeps
        // happy-path tests that send-then-cancel from losing the tail.
        while let Ok(window) = self.rx.try_recv() {
            self.sink.write(&window).map_err(map_sink_err)?;
        }

        let artifact = self.sink.finalize().map_err(map_sink_err)?;
        Ok(artifact)
    }
}

/// Map a [`RecordingSinkError`] into the worker-level [`RecordingError`],
/// promoting `ErrorKind::StorageFull` (and the stable `ENOSPC` raw OS code)
/// into the typed [`RecordingError::DiskFull`] so the IPC boundary can
/// surface a machine-readable disk-full signal to the front-end without
/// substring-matching on `format!`.
fn map_sink_err(e: RecordingSinkError) -> RecordingError {
    if let RecordingSinkError::Io(ref io) = e {
        // `ErrorKind::StorageFull` is the canonical kind in newer std
        // releases; older targets may surface ENOSPC only via raw_os_error.
        if io.kind() == std::io::ErrorKind::StorageFull {
            return RecordingError::DiskFull;
        }
        // ENOSPC = 28 on Linux/macOS; ERROR_DISK_FULL = 112 on Windows.
        // Match both so the typed variant is reachable across platforms.
        if let Some(code) = io.raw_os_error() {
            #[cfg(unix)]
            if code == 28 {
                return RecordingError::DiskFull;
            }
            #[cfg(windows)]
            if code == 112 {
                return RecordingError::DiskFull;
            }
            // Suppress unused-variable warning on platforms where neither
            // arm matches.
            let _ = code;
        }
    }
    RecordingError::Sink(e)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn mock_sink_round_trips_samples_and_artifact() {
        let path = std::env::temp_dir().join("mock_unit.flac");
        let mut sink = Box::new(MockRecordingSink::new(path, 48_000));
        sink.write(&[0.0, 0.5, -0.5]).expect("write");
        let artifact = sink.finalize().expect("finalize");
        assert_eq!(artifact.sample_count, 3);
        assert_eq!(artifact.sample_rate_hz, 48_000);
    }

    #[test]
    fn mock_sink_double_finalize_errors() {
        let tmp = std::env::temp_dir();
        let mut sink = Box::new(MockRecordingSink::new(tmp.join("mock_double.flac"), 48_000));
        sink.write(&[0.0]).expect("write");
        let _artifact = Box::new(MockRecordingSink::new(
            tmp.join("mock_double_other.flac"),
            48_000,
        ))
        .finalize()
        .expect("first finalize");
        // Single-shot finalize is enforced by `Box<Self>` consuming the box;
        // there is no way to call `finalize` twice on the same instance from
        // safe code. Double-finalize is therefore covered by the type system.
        let mut sink2 = MockRecordingSink::new(tmp.join("mock_double2.flac"), 48_000);
        sink2.finalized = true;
        let res = Box::new(sink2).finalize();
        assert!(matches!(res, Err(RecordingSinkError::AlreadyFinalized)));
    }

    #[cfg(feature = "flac")]
    #[test]
    fn flac_sink_rejects_non_48k() {
        let res =
            FlacRecordingSink::create(std::env::temp_dir().join("should_not_create.flac"), 44_100);
        assert!(matches!(res, Err(RecordingSinkError::InvalidConfig(_))));
    }
}
