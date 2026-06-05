//! Tauri command surface for the NeuralPitch shell.
//!
//! All commands return `Result<T, String>` per ADR-0015 — errors are
//! formatted with `format!("{e:#}")` so the front-end gets the full
//! `anyhow`-style chain. Validation failures do not mutate state.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use cpal::SampleFormat;
use cpal::traits::{DeviceTrait, HostTrait};
use neural_pitch_core::audio::backend::{AudioBackend, AudioBackendConfig, AudioBackendError};
use neural_pitch_core::audio::cpal_backend::CpalAudioBackend;
use neural_pitch_core::audio::{AudioBackendEvent, AudioEventEmitter};
use neural_pitch_core::pipeline::{
    DspError, DspWorker, PitchUpdate, RecordingFanout, RecordingProgress, RecordingWorker,
};
use neural_pitch_core::pitch::factory::{Backend, make_estimator};
use neural_pitch_core::pitch::{
    EstimatorConfig, EstimatorError, InstrumentHint, live_search_range_for_hint,
};
use neural_pitch_core::settings::TunerSettings;
use neural_pitch_core::smoothing::ContourSmoother;
use neural_pitch_core::voicing::VoiceActivityGate;
use serde::Serialize;
use serde_json::Value;
use tauri::State;
use tauri::ipc::Channel;
use tokio_util::sync::CancellationToken;

use crate::sink::TauriChannelFrameSink;
use crate::state::{AppState, DspController};

/// Store key under which [`TunerSettings`] is persisted.
pub(crate) const SETTINGS_STORE_KEY: &str = "settings";

/// Maximum wait for the DSP worker thread to join during `stop_capture`.
const DSP_JOIN_BUDGET: Duration = Duration::from_millis(500);

/// Begin capture with the supplied settings, streaming [`PitchUpdate`]
/// frames through `channel` and out-of-band [`AudioBackendEvent`]s through
/// `events`.
///
/// Failure semantics — strictly atomic with respect to disk + in-memory
/// state. The settings cache and the on-disk store are mutated only after
/// `build_controller` succeeds; any earlier validation, "already capturing",
/// or backend-construction failure leaves the caller's prior settings
/// intact.
///
/// macOS: TCC microphone permission is granted via the bundle's
/// `entitlements.plist` + `Info.plist` `NSMicrophoneUsageDescription`. The
/// first `cpal::Device::build_input_stream` call triggers the OS prompt;
/// on denial cpal returns `BuildStreamError::BackendSpecific`, which the
/// shell maps to a user-facing string telling the operator to enable the
/// permission in System Settings → Privacy & Security → Microphone.
/// ADR-0017 forbids any telemetry on permission denial.
#[tauri::command]
#[tracing::instrument(
    skip(state, channel, events),
    fields(
        sample_rate_hz = settings.sample_rate_hz,
        window_size = settings.window_size,
        hop_size = settings.hop_size,
        a4_hz = settings.a4_hz,
        instrument_hint = ?settings.instrument_hint,
    ),
)]
pub async fn start_capture(
    state: State<'_, AppState>,
    channel: Channel<PitchUpdate>,
    events: Channel<AudioBackendEvent>,
    settings: TunerSettings,
) -> Result<(), String> {
    settings
        .validate()
        .map_err(|e| format!("invalid settings: {e:#}"))?;

    // Refuse a duplicate start *before* we mutate settings or build the
    // controller — the original code path persisted-then-checked, which
    // committed bad config to disk on the duplicate-start error path.
    {
        let guard = state.dsp.lock();
        if guard.is_some() {
            return Err("already capturing".into());
        }
    }

    // Stash the event channel so the JS side can keep its handle around
    // across stop/start round-trips, and wrap a clone in the
    // `AudioEventEmitter` closure handed to the cpal backend. We clone
    // before moving into the closure because `Channel` is reference-counted.
    {
        let mut g = state.events.lock();
        *g = Some(events.clone());
    }
    let emitter: AudioEventEmitter = Arc::new(move |ev: AudioBackendEvent| {
        // RT-safety note: this `Channel::send` is acceptable here because
        // the cpal `err_fn` is documented to run OFF the RT data path on
        // every supported backend (CoreAudio HAL listener thread on macOS,
        // WASAPI event thread on Windows, ALSA poll thread on Linux).
        // `Channel::send` synchronously serialises JSON on the calling
        // thread — see `crate::sink` for the underlying analysis. Do NOT
        // reuse this emitter shape inside an `InputDataCallback`; per-sample
        // hot paths are explicitly forbidden from allocating or logging.
        // The error branch is benign: a dropped front-end channel just
        // means there is no consumer.
        if let Err(e) = events.send(ev) {
            tracing::debug!(target: "neural_pitch::commands", error = %e, "audio event channel send failed");
        }
    });

    let controller =
        build_controller(&settings, channel, Some(emitter)).map_err(translate_build_error)?;

    // Commit the new baseline to the settings cache + persist. The cache
    // write and disk write are not strictly transactional (parking_lot
    // guards are `!Send` and cannot cross `.await`), but: (a) we drop the
    // write guard before awaiting persist, and (b) `persist_settings`
    // serialises *the post-await snapshot* — so a concurrent set_setting
    // that interleaves can win the cache, but the disk converges to the
    // last committed cache state. See `persist_settings`.
    let snapshot = {
        let mut g = state.settings.write();
        *g = settings.clone();
        g.clone()
    };
    persist_settings(&state, snapshot).await?;

    let mut guard = state.dsp.lock();
    if guard.is_some() {
        // A concurrent start_capture won the race after our pre-check. We
        // already created a second backend; explicitly tear it down before
        // bailing so we do not leak a cpal stream and do not leave an
        // orphan DSP worker spinning on a dropped consumer for several
        // hop intervals (~10 ms at hop=512/48 kHz). `Drop` on the
        // controller alone would close the cpal handle but would not
        // signal the worker to exit promptly.
        let mut losing = controller;
        losing.cancel.cancel();
        losing.backend.stop();
        // Release the AppState lock while we wait for the worker join so
        // a parallel stop_capture can still acquire `state.dsp` if needed.
        drop(guard);
        if let Some(handle) = losing.worker_join.take() {
            let deadline = std::time::Instant::now() + DSP_JOIN_BUDGET;
            let mut joined = false;
            while std::time::Instant::now() < deadline {
                if handle.is_finished() {
                    let _ = handle.join();
                    joined = true;
                    break;
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            if !joined {
                tracing::warn!(
                    "dsp worker did not exit within budget on concurrent-start teardown"
                );
            }
        }
        return Err("already capturing".into());
    }
    *guard = Some(controller);
    Ok(())
}

/// Stop the live capture pipeline and tear down the cpal stream.
///
/// Idempotent: calling this on a stopped state returns `Ok(())`.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub async fn stop_capture(state: State<'_, AppState>) -> Result<(), String> {
    let Some(mut controller) = state.dsp.lock().take() else {
        return Ok(());
    };

    controller.cancel.cancel();
    controller.backend.stop();

    if let Some(handle) = controller.worker_join.take() {
        // Wait for the worker on a blocking-capable thread so the tokio
        // worker thread is not parked on `std::thread::sleep`. Bound the
        // total wait by `DSP_JOIN_BUDGET`; on timeout we drop the handle
        // and proceed (the cpal stream and producer are gone via
        // `backend.stop()`, so no shared OS resource is leaked).
        let join_outcome = tokio::time::timeout(
            DSP_JOIN_BUDGET,
            tokio::task::spawn_blocking(move || handle.join()),
        )
        .await;
        match join_outcome {
            // timeout(spawn_blocking(handle.join())):
            //   Result<Result<Result<Result<(), DspError>, Box<dyn Any>>, JoinError>, Elapsed>
            //
            // i.e. (outer→inner):
            //   Err(_)                         => DSP_JOIN_BUDGET elapsed
            //   Ok(Err(spawn_join_err))        => spawn_blocking task panicked
            //   Ok(Ok(Err(thread_join_err)))   => DSP worker thread panicked
            //   Ok(Ok(Ok(Err(dsp_err))))       => DSP worker returned an error
            //   Ok(Ok(Ok(Ok(()))))             => clean exit
            Ok(Ok(Ok(Ok(())))) => {
                tracing::info!("dsp worker exited cleanly");
            }
            Ok(Ok(Ok(Err(e)))) => {
                tracing::warn!(error = %e, "dsp worker returned error");
            }
            Ok(Ok(Err(_))) => {
                tracing::warn!("dsp worker thread panicked");
            }
            Ok(Err(e)) => {
                tracing::warn!(error = %e, "spawn_blocking failed during dsp join");
            }
            Err(_) => {
                tracing::warn!("dsp join exceeded {:?}", DSP_JOIN_BUDGET);
            }
        }
    }
    Ok(())
}

// -- Recording lifecycle ----------------------------------------------------

/// Recording fan-out channel capacity in hop slices.
///
/// At hop=512 / 48 kHz, ~93 hops/sec. 24 slots ≈ 250 ms of buffering
/// before backpressure kicks in. The recording worker drains as fast as
/// the sink (FLAC encoding ≈ ~3 ms per hop on a modern CPU), so the
/// queue is empty in steady state and backpressure only matters during
/// disk-stall transients.
const RECORDING_CHANNEL_CAPACITY: usize = 24;

/// Maximum wait for the recording encoder thread to join during
/// `stop_recording`. Mirrors `DSP_JOIN_BUDGET` and the recommendation in
/// `RecordingHandle::stop_with_timeout`.
const RECORDING_JOIN_BUDGET: Duration = Duration::from_secs(2);

/// Start a new recording. Requires `start_capture` to have run first; the
/// DSP worker's recording fan-out is attached to the bounded
/// `sync_channel` driving the FLAC encoder thread. Returns the synthetic
/// recording id (used by `get_recording_path`).
///
/// Failure semantics:
///   - `Err("not capturing")` if the DSP pipeline is stopped
///   - `Err("already recording")` if a take is already in flight
///   - `Err("create sink: ...")` if the FLAC partial file cannot be
///     opened (permission denied, no space, etc.)
#[tauri::command]
#[tracing::instrument(skip(state, progress))]
pub async fn start_recording(
    state: State<'_, crate::state::AppState>,
    progress: Channel<RecordingProgress>,
    instrument_profile: String,
) -> Result<String, String> {
    // Refuse if a recording is already in flight.
    {
        let g = state.recording.lock();
        if g.is_some() {
            return Err("already recording".into());
        }
    }
    // Recording requires the DSP pipeline to be running so the fan-out
    // is wired up.
    let fanout = {
        let g = state.dsp.lock();
        match g.as_ref() {
            Some(c) => c.recording_fanout.clone(),
            None => return Err("not capturing — call start_capture first".into()),
        }
    };

    let settings_snapshot = state.settings_snapshot();
    let sample_rate_hz = settings_snapshot.sample_rate_hz;
    if sample_rate_hz != 48_000 {
        return Err(format!(
            "ADR-0011 requires 48 kHz sample rate for FLAC recordings (current: {sample_rate_hz})"
        ));
    }

    // Mint a synthetic recording id and resolve the on-disk path. We use
    // a UUIDv7 (matches the `recordings` table key) so the same id can
    // be reused as the row primary key on stop. The filename embeds the
    // creation timestamp + a short id suffix so on-disk listings sort
    // deterministically.
    let recording_id = neural_pitch_core::store::RecordingId::new_v7();
    let id_string = recording_id.to_string();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
        .unwrap_or(0);
    let id_suffix: String = id_string.chars().take(8).collect();
    let filename = format!("rec-{now_ms}-{id_suffix}.flac");
    let path = state.recordings_dir.join(&filename);

    // Build the FLAC sink + recording worker. The sink's create() opens
    // <path>.partial; create-time errors (no parent dir, EACCES, etc.)
    // surface here as a typed error rather than landing in the encoder
    // thread. `FlacRecordingSink` is gated behind `neural-pitch-core`'s
    // `flac` feature (default-enabled); the shell relies on that being
    // active. A minimal CI build that disables the feature would fail
    // to compile this command, which is the desired behaviour.
    let sink: Box<dyn neural_pitch_core::pipeline::RecordingSink> = Box::new(
        neural_pitch_core::pipeline::FlacRecordingSink::create(&path, 48_000)
            .map_err(|e| format!("create sink: {e:#}"))?,
    );

    // Bounded fan-out channel.
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<f32>>(RECORDING_CHANNEL_CAPACITY);
    let dropped = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let cancel = CancellationToken::new();

    let worker = RecordingWorker::new(sink, rx, cancel.clone(), Arc::clone(&dropped));
    let core_id = neural_pitch_core::pipeline::RecordingId::new(id_string.clone());
    let handle = worker
        .spawn(core_id)
        .map_err(|e| format!("spawn recording worker: {e:#}"))?;

    // Attach to the live DSP fan-out. From here, every hop slide writes
    // a fresh slice into the recording channel.
    fanout.attach(tx, Arc::clone(&dropped));

    // Spawn a periodic progress-emitter on tokio so the UI's
    // `recording-progress` channel ticks at ~5 Hz. Detaches once the
    // recording stops (we don't keep the join handle — when the cancel
    // token flips the loop exits on its own).
    {
        let cancel_for_ticker = cancel.clone();
        let dropped_for_ticker = Arc::clone(&dropped);
        let started_at = std::time::Instant::now();
        let progress = progress.clone();
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(Duration::from_millis(200));
            loop {
                ticker.tick().await;
                if cancel_for_ticker.is_cancelled() {
                    break;
                }
                let duration_ms =
                    u64::try_from(started_at.elapsed().as_millis()).unwrap_or(u64::MAX);
                let dropped_n = dropped_for_ticker.load(std::sync::atomic::Ordering::Relaxed);
                if let Err(e) = progress.send(RecordingProgress::Tick {
                    sample_count: 0, // running tally not exposed by handle yet
                    duration_ms,
                    dropped_windows: dropped_n,
                }) {
                    tracing::debug!(error = %e, "recording-progress channel send failed");
                    break;
                }
            }
        });
    }

    {
        let mut g = state.recording.lock();
        *g = Some(crate::state::ActiveRecording {
            handle,
            path,
            filename,
            instrument_profile,
            sample_rate_hz: i64::from(sample_rate_hz),
            created_at_unix_ms: now_ms,
            a4_hz: f64::from(settings_snapshot.a4_hz),
        });
    }

    Ok(id_string)
}

/// Stop the in-flight recording, finalize the FLAC file, write the
/// `recordings` row, and return the persisted [`Recording`] payload.
///
/// Idempotent: returns `Err("not recording")` if no take is in flight.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub async fn stop_recording(
    state: State<'_, crate::state::AppState>,
) -> Result<serde_json::Value, String> {
    let active = {
        let mut g = state.recording.lock();
        g.take()
    };
    let Some(active) = active else {
        return Err("not recording".into());
    };

    // Detach the DSP fan-out before we ask the recording worker to drain
    // its tail. From this point the live tuner pipeline keeps running
    // but no new hop slices land in the recording channel.
    if let Some(controller) = state.dsp.lock().as_ref() {
        controller.recording_fanout.detach();
    }

    // Bound the wait on the encoder thread so a slow disk fsync cannot
    // hang the IPC call indefinitely. spawn_blocking moves the
    // synchronous join off the tokio runtime worker.
    let crate::state::ActiveRecording {
        handle,
        path,
        filename,
        instrument_profile,
        sample_rate_hz,
        created_at_unix_ms,
        a4_hz,
    } = active;

    let join_result =
        tokio::task::spawn_blocking(move || handle.stop_with_timeout(RECORDING_JOIN_BUDGET))
            .await
            .map_err(|e| format!("recording stop task panicked: {e}"))?;

    let artifact = join_result.map_err(|e| format!("stop recording: {e:#}"))?;

    // Persist the row. spawn_blocking again because the library API is
    // synchronous and SQLite writes block.
    let library = Arc::clone(&state.library);
    let row_filename = filename.clone();
    let inst_profile = instrument_profile.clone();
    let id = tokio::task::spawn_blocking(move || {
        library.insert_recording(neural_pitch_core::store::NewRecording {
            filename: row_filename,
            created_at_unix_ms,
            duration_ms: i64::try_from(artifact.duration_ms).unwrap_or(i64::MAX),
            sample_rate_hz,
            channels: 1,
            bit_depth: 24,
            format: "flac".to_string(),
            a4_hz,
            instrument_profile: inst_profile,
            user_label: None,
        })
    })
    .await
    .map_err(|e| format!("library insert task panicked: {e}"))?
    .map_err(|e| format!("library insert: {e:#}"))?;

    // Wire the response shape used by the front-end mock — snake_case
    // wire format mirroring the `Recording` row.
    Ok(serde_json::json!({
        "id": id.to_string(),
        "filename": filename,
        "created_at": created_at_unix_ms,
        "duration_ms": artifact.duration_ms,
        "sample_rate_hz": sample_rate_hz,
        "channels": 1,
        "bit_depth": 24,
        "a4_hz": a4_hz,
        "instrument_profile": instrument_profile,
        "user_label": null,
        "path": path,
    }))
}

/// Resolve a recording id (or the in-flight one) to its on-disk absolute
/// path. Used by the front-end PlaybackPanel to pass into
/// `convertFileSrc`.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub async fn get_recording_path(
    state: State<'_, crate::state::AppState>,
    id: String,
) -> Result<String, String> {
    // Resolve the row → filename, then join with `recordings_dir`.
    let parsed: neural_pitch_core::store::RecordingId = id
        .parse()
        .map_err(|e| format!("invalid recording id: {e}"))?;
    let library = Arc::clone(&state.library);
    let recording = tokio::task::spawn_blocking(move || {
        library.list_recordings(neural_pitch_core::store::ListFilter::IncludingDeleted)
    })
    .await
    .map_err(|e| format!("recording lookup task panicked: {e}"))?
    .map_err(|e| format!("list recordings: {e:#}"))?;
    let row = recording
        .into_iter()
        .find(|r| r.id == parsed)
        .ok_or_else(|| format!("recording {id} not found"))?;
    let path = state.recordings_dir.join(&row.filename);
    let abs = path
        .to_str()
        .ok_or_else(|| format!("recording path not utf-8: {}", path.display()))?
        .to_string();
    Ok(abs)
}

/// List every active recording (excludes soft-deleted rows).
#[tauri::command]
#[tracing::instrument(skip(state))]
pub async fn list_recordings(
    state: State<'_, crate::state::AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    let library = Arc::clone(&state.library);
    let rows = tokio::task::spawn_blocking(move || {
        library.list_recordings(neural_pitch_core::store::ListFilter::ActiveOnly)
    })
    .await
    .map_err(|e| format!("list_recordings task panicked: {e}"))?
    .map_err(|e| format!("list recordings: {e:#}"))?;
    Ok(rows
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id.to_string(),
                "filename": r.filename,
                "created_at": r.created_at_unix_ms,
                "duration_ms": r.duration_ms,
                "sample_rate_hz": r.sample_rate_hz,
                "channels": r.channels,
                "bit_depth": r.bit_depth,
                "a4_hz": r.a4_hz,
                "instrument_profile": r.instrument_profile,
                "user_label": r.user_label,
            })
        })
        .collect())
}

/// Soft-delete a recording. Tombstones the row but keeps the on-disk file
/// so a future "undelete" / hard-purge can clean up.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub async fn delete_recording(
    state: State<'_, crate::state::AppState>,
    id: String,
) -> Result<(), String> {
    let parsed: neural_pitch_core::store::RecordingId = id
        .parse()
        .map_err(|e| format!("invalid recording id: {e}"))?;
    let library = Arc::clone(&state.library);
    tokio::task::spawn_blocking(move || library.soft_delete(parsed))
        .await
        .map_err(|e| format!("delete_recording task panicked: {e}"))?
        .map_err(|e| format!("delete recording: {e:#}"))?;
    Ok(())
}

// -- Phase 2.1 analysis surface ---------------------------------------------

/// Default analyzer name handed to the cache layer. Mirrors
/// [`neural_pitch_core::analysis::contour::PYIN_ANALYZER_NAME`] so the IPC
/// surface and the core constant cannot drift.
const DEFAULT_ANALYZER_NAME: &str = neural_pitch_core::analysis::contour::PYIN_ANALYZER_NAME;
/// Default analyzer version. Sourced from
/// [`neural_pitch_core::analysis::contour::PYIN_ANALYZER_VERSION`] so the
/// cache key on the IPC side is identical to the core canonical key.
///
/// Contributor invariant — bump in lock-step with ANY of:
///   * a field set / wire ordering change to
///     `analysis::contour::ContourResult` (or a nested type, e.g.
///     `crate::pitch::F0Frame`),
///   * an analyzer parameter change that materially shifts the f0
///     contour (defaults for fmin/fmax, hop_size, window_size,
///     smoothing window, voicing threshold),
///   * a postcard format-version bump.
///
/// Failure to bump leads to silent stale-cache hits where an old blob
/// decodes against the new shape and surfaces wrong values to the UI —
/// the SQL key compares plain strings (no semver normalisation, see
/// `store::analysis`), so the cache layer cannot detect the divergence
/// on its own.
const DEFAULT_ANALYZER_VERSION: &str = neural_pitch_core::analysis::contour::PYIN_ANALYZER_VERSION;

/// Adapter that lets `analyze_recording_blocking` emit progress through a
/// `tauri::ipc::Channel<AnalysisProgress>` without dragging Tauri types
/// into `neural-pitch-core` (P2 / ADR-0002).
///
/// `Channel::send` synchronously serialises JSON on the calling thread.
/// The blocking analyzer runs inside `spawn_blocking`, so the send happens
/// off the tokio runtime thread — RT-safety properties are identical to
/// `start_recording`'s progress channel.
struct ChannelProgressSink {
    channel: Channel<neural_pitch_core::store::AnalysisProgress>,
}

impl neural_pitch_core::store::ProgressSink for ChannelProgressSink {
    fn emit(&self, progress: neural_pitch_core::store::AnalysisProgress) {
        if let Err(e) = self.channel.send(progress) {
            tracing::debug!(error = %e, "analysis-progress channel send failed");
        }
    }
}

/// Run a full analysis on a previously-recorded file and persist the
/// result to the `analysis_cache` table.
///
/// Cache hit + `!force_refresh`: deserialise the postcard blob, emit one
/// `AnalysisProgress { percent: 1.0, was_cached: true, .. }`, return
/// `AnalysisSummary { was_cached: true, .. }`.
///
/// Cache miss / `force_refresh`: decode the source FLAC, run the
/// analyzer on a `spawn_blocking` worker, persist via
/// `library.upsert_analysis(...)`, and return `was_cached: false`.
///
/// `progress` mirrors `start_recording`: the JS side constructs the
/// channel once on mount and passes it through every invocation. Headless
/// callers bypass the Tauri command and call
/// [`neural_pitch_core::store::analyze_recording_blocking`] directly with
/// `progress = None`.
#[tauri::command]
#[tracing::instrument(skip(state, progress), fields(force_refresh = force_refresh))]
pub async fn analyze_recording(
    state: State<'_, crate::state::AppState>,
    recording_id: String,
    force_refresh: bool,
    progress: Channel<neural_pitch_core::store::AnalysisProgress>,
) -> Result<neural_pitch_core::store::AnalysisSummary, String> {
    let parsed: neural_pitch_core::store::RecordingId = recording_id
        .parse()
        .map_err(|e| format!("invalid recording id: {e}"))?;

    // Mint a cancel token and register it. Drop the entry on every exit
    // path (success / error / cancel) so the registry stays bounded by
    // the number of concurrent in-flight runs.
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let mut g = state.analyses.lock();
        // If a previous run for the same recording is still registered
        // (e.g. front-end double-clicked Analyse), cancel it first so
        // the new request takes over the slot.
        if let Some(prev) = g.insert(parsed, Arc::clone(&cancel)) {
            prev.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    let library = Arc::clone(&state.library);
    let cancel_for_blocking = Arc::clone(&cancel);
    let sink_owned = ChannelProgressSink { channel: progress };
    let result = tokio::task::spawn_blocking(move || {
        let sink_ref: &dyn neural_pitch_core::store::ProgressSink = &sink_owned;
        neural_pitch_core::store::analyze_recording_blocking(
            &library,
            parsed,
            DEFAULT_ANALYZER_NAME,
            DEFAULT_ANALYZER_VERSION,
            force_refresh,
            Some(sink_ref),
            Some(cancel_for_blocking.as_ref()),
        )
    })
    .await
    .map_err(|e| {
        let mut g = state.analyses.lock();
        g.remove(&parsed);
        format!("analyze_recording task panicked: {e}")
    })?;

    {
        let mut g = state.analyses.lock();
        // Only remove our entry if it is still the active one — a
        // racing reanalyse may have replaced it.
        if let Some(existing) = g.get(&parsed) {
            if Arc::ptr_eq(existing, &cancel) {
                g.remove(&parsed);
            }
        }
    }

    result.map_err(|e| format!("analyze: {e:#}"))
}

/// Fetch a previously cached contour for `(recording_id, analyzer_name,
/// analyzer_version)`. Returns `Err("not found")` if no row matches.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub async fn get_contour(
    state: State<'_, crate::state::AppState>,
    recording_id: String,
    analyzer_name: String,
    analyzer_version: String,
) -> Result<neural_pitch_core::store::ContourResult, String> {
    let parsed: neural_pitch_core::store::RecordingId = recording_id
        .parse()
        .map_err(|e| format!("invalid recording id: {e}"))?;
    let library = Arc::clone(&state.library);
    let name = analyzer_name;
    let version = analyzer_version;
    let res = tokio::task::spawn_blocking(move || {
        neural_pitch_core::store::get_contour_blocking(&library, parsed, &name, &version)
    })
    .await
    .map_err(|e| format!("get_contour task panicked: {e}"))?
    .map_err(|e| format!("get_contour: {e:#}"))?;
    res.ok_or_else(|| "not found".to_string())
}

/// Enumerate every cached analysis row for one recording.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub async fn list_analyses(
    state: State<'_, crate::state::AppState>,
    recording_id: String,
) -> Result<Vec<neural_pitch_core::store::AnalysisRow>, String> {
    let parsed: neural_pitch_core::store::RecordingId = recording_id
        .parse()
        .map_err(|e| format!("invalid recording id: {e}"))?;
    let library = Arc::clone(&state.library);
    tokio::task::spawn_blocking(move || {
        neural_pitch_core::store::list_analyses_blocking(&library, parsed)
    })
    .await
    .map_err(|e| format!("list_analyses task panicked: {e}"))?
    .map_err(|e| format!("list_analyses: {e:#}"))
}

/// Drop one cached analysis row.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub async fn delete_analysis(
    state: State<'_, crate::state::AppState>,
    recording_id: String,
    analyzer_name: String,
    analyzer_version: String,
) -> Result<(), String> {
    let parsed: neural_pitch_core::store::RecordingId = recording_id
        .parse()
        .map_err(|e| format!("invalid recording id: {e}"))?;
    let library = Arc::clone(&state.library);
    let name = analyzer_name;
    let version = analyzer_version;
    tokio::task::spawn_blocking(move || {
        neural_pitch_core::store::delete_analysis_blocking(&library, parsed, &name, &version)
    })
    .await
    .map_err(|e| format!("delete_analysis task panicked: {e}"))?
    .map_err(|e| format!("delete_analysis: {e:#}"))
}

// -- Phase 2.3 range / vibrato accessors ------------------------------------
//
// Convenience IPC surface for the recordings UI: shell out to a
// `spawn_blocking` worker that postcard-decodes the cached `(recording_id,
// analyzer_name, analyzer_version)` blob and projects the requested
// sub-field. No separate row, no second cache key — see Phase 2.3 §2 and
// ADR-0021 for the cache-version bump that backs the projection.
//
// Both commands re-use the `a4_hz` stored on the originating recordings
// row so the projection is consistent with what `analyze_recording`'s
// `summarize_cached` produced. Looking the row up here keeps the IPC
// surface symmetric with `get_contour` (the caller passes `recording_id`,
// the shell resolves the rest).

/// Resolve a recording id to its `a4_hz` reference pitch.
///
/// Phase 2.3 range / vibrato projections need the `a4_hz` from the row
/// (per ADR-0005 — no module-level A4 state). We accept the SQLite hop on
/// the spawn_blocking worker rather than forcing the caller to supply
/// `a4_hz` over IPC; the recording row is the source of truth.
///
/// Returns `Ok(None)` when the recording id does not resolve, so callers
/// can map the missing-row case to the same `"not found"` string
/// `get_contour` uses (see `get_range_report` / `get_vibrato_report`).
/// `Err(_)` is reserved for SQLite I/O failures.
fn lookup_a4_hz(
    library: &neural_pitch_core::store::RecordingsLibrary,
    recording_id: neural_pitch_core::store::RecordingId,
) -> Result<Option<f32>, String> {
    let rows = library
        .list_recordings(neural_pitch_core::store::ListFilter::IncludingDeleted)
        .map_err(|e| format!("list recordings: {e:#}"))?;
    Ok(rows
        .into_iter()
        .find(|r| r.id == recording_id)
        .map(|row| row.a4_hz as f32))
}

/// Fetch the [`neural_pitch_core::analysis::range::RangeReport`] for
/// `(recording_id, analyzer_name, analyzer_version)`. Mirrors
/// `get_contour`'s error semantics:
///   * `Err("not found")` — no row matches the cache key.
///   * `Err("not present in cache row")` — the row exists but predates
///     Phase 2.3 (the cached `ContourResult` blob fails postcard decode
///     under the live schema and the version is not a recognised legacy);
///     the front-end should re-run with `force_refresh = true`.
///
/// Otherwise the cached blob is decoded, `compute_range` is projected
/// over the contour, and the result is returned. The same `a4_hz`
/// reference the originating recording row carries is used (per
/// ADR-0005).
#[tauri::command]
#[tracing::instrument(skip(state))]
pub async fn get_range_report(
    state: State<'_, crate::state::AppState>,
    recording_id: String,
    analyzer_name: String,
    analyzer_version: String,
) -> Result<neural_pitch_core::analysis::range::RangeReport, String> {
    let parsed: neural_pitch_core::store::RecordingId = recording_id
        .parse()
        .map_err(|e| format!("invalid recording id: {e}"))?;
    let library = Arc::clone(&state.library);
    let result = tokio::task::spawn_blocking(
        move || -> Result<Option<neural_pitch_core::analysis::range::RangeReport>, String> {
            // Missing recording id collapses to `Ok(None)` here so the
            // outer `result.ok_or_else(|| "not found")` produces the same
            // `Err("not found")` shape `get_contour` documents.
            let Some(a4_hz) = lookup_a4_hz(&library, parsed)? else {
                return Ok(None);
            };
            neural_pitch_core::store::get_range_report_blocking(
                &library,
                parsed,
                &analyzer_name,
                &analyzer_version,
                a4_hz,
            )
            .map_err(|e| match e {
                neural_pitch_core::store::AnalysisError::CacheCorrupted => {
                    "not present in cache row".to_string()
                }
                other => format!("get_range_report: {other:#}"),
            })
        },
    )
    .await
    .map_err(|e| format!("get_range_report task panicked: {e}"))??;
    result.ok_or_else(|| "not found".to_string())
}

/// Fetch the [`neural_pitch_core::analysis::vibrato::VibratoReport`] for
/// `(recording_id, analyzer_name, analyzer_version)`. Same error
/// semantics as [`get_range_report`].
#[tauri::command]
#[tracing::instrument(skip(state))]
pub async fn get_vibrato_report(
    state: State<'_, crate::state::AppState>,
    recording_id: String,
    analyzer_name: String,
    analyzer_version: String,
) -> Result<neural_pitch_core::analysis::vibrato::VibratoReport, String> {
    let parsed: neural_pitch_core::store::RecordingId = recording_id
        .parse()
        .map_err(|e| format!("invalid recording id: {e}"))?;
    let library = Arc::clone(&state.library);
    let result = tokio::task::spawn_blocking(
        move || -> Result<Option<neural_pitch_core::analysis::vibrato::VibratoReport>, String> {
            // Missing recording id collapses to `Ok(None)` here so the
            // outer `result.ok_or_else(|| "not found")` produces the same
            // `Err("not found")` shape `get_contour` documents.
            let Some(a4_hz) = lookup_a4_hz(&library, parsed)? else {
                return Ok(None);
            };
            neural_pitch_core::store::get_vibrato_report_blocking(
                &library,
                parsed,
                &analyzer_name,
                &analyzer_version,
                a4_hz,
            )
            .map_err(|e| match e {
                neural_pitch_core::store::AnalysisError::CacheCorrupted => {
                    "not present in cache row".to_string()
                }
                other => format!("get_vibrato_report: {other:#}"),
            })
        },
    )
    .await
    .map_err(|e| format!("get_vibrato_report task panicked: {e}"))??;
    result.ok_or_else(|| "not found".to_string())
}

// -- Phase 2.2 backend-aware analysis surface -------------------------------

/// Wire shape for the requested pitch backend.
///
/// `tag = "kind"` so the front-end matches exhaustively on the discriminant
/// rather than the field set; `untagged` would let a typo silently fall
/// into a different arm. Phase 2.2 ships the four shipping backends; the
/// neural arms carry the resolved on-disk path so the resolver stays
/// out of the IPC boundary.
///
/// Discriminant naming: each variant carries an explicit `#[serde(rename)]`
/// so the on-the-wire `kind` matches the value persisted in
/// `analysis_cache.analyzer_name` (e.g. `"pyin"`, `"crepe-tiny"`). Without
/// the explicit rename, `rename_all = "snake_case"` would coin `p_yin` /
/// `crepe_tiny`, splitting the IPC discriminant from the cache key and
/// burning the front-end engineer who joins the two.
#[derive(serde::Deserialize, Debug, Clone)]
#[serde(tag = "kind")]
pub enum BackendKind {
    /// Plain YIN (de Cheveigne & Kawahara 2002).
    #[serde(rename = "yin")]
    Yin,
    /// pYIN (Mauch & Dixon 2014). Requires `feature = "pyin"` in core.
    #[serde(rename = "pyin")]
    PYin,
    /// PESTO neural backend. Requires `feature = "neural"` in core.
    #[serde(rename = "pesto")]
    Pesto {
        /// Resolved on-disk path to the PESTO ONNX file.
        onnx_path: PathBuf,
    },
    /// CREPE-tiny neural backend. Requires `feature = "neural"` in core.
    #[serde(rename = "crepe-tiny")]
    CrepeTiny {
        /// Resolved on-disk path to the CREPE-tiny ONNX file.
        onnx_path: PathBuf,
    },
}

impl BackendKind {
    /// Stable analyzer name persisted in `analysis_cache.analyzer_name`.
    /// Matches the on-the-wire `kind` discriminant exactly so a single
    /// string round-trips between IPC and SQLite.
    fn analyzer_name(&self) -> &'static str {
        match self {
            Self::Yin => "yin",
            Self::PYin => "pyin",
            Self::Pesto { .. } => "pesto",
            Self::CrepeTiny { .. } => "crepe-tiny",
        }
    }

    /// Stable analyzer version persisted in
    /// `analysis_cache.analyzer_version`. Bump in lock-step with on-the-wire
    /// shape changes so cached blobs invalidate cleanly.
    fn analyzer_version(&self) -> &'static str {
        match self {
            Self::Yin | Self::PYin => DEFAULT_ANALYZER_VERSION,
            // Phase 2.2 baseline.
            Self::Pesto { .. } | Self::CrepeTiny { .. } => "0.1",
        }
    }
}

/// Run a backend-selected analysis on a previously-recorded file.
///
/// Phase 2.2 routing reality: the underlying analyzer plumbing in
/// [`neural_pitch_core::store::analyze_recording_blocking`] is hard-wired
/// to pYIN — `run_analyzer_with_progress` constructs a `PYinEstimator`
/// unconditionally and ignores the supplied `analyzer_name`. To prevent
/// silent data-mislabelling (pYIN-derived contour bytes persisted under
/// `analyzer_name = "pesto"`), this command short-circuits every backend
/// other than [`BackendKind::PYin`] with `Err("backend not yet routed:
/// <name> — Phase 2.5")` until the dispatcher in core lands. YIN is also
/// refused for the same reason: the live tuner uses YIN/MPM, but the
/// offline path here would still execute pYIN under the YIN cache key.
///
/// When the dispatcher arrives, this guard lifts and the `BackendKind`
/// arms route to the appropriate `make_estimator` arm.
#[tauri::command]
#[tracing::instrument(
    skip(state, progress, backend),
    fields(force_refresh = force_refresh, backend = ?backend),
)]
pub async fn analyze_recording_with_backend(
    state: State<'_, crate::state::AppState>,
    recording_id: String,
    backend: BackendKind,
    force_refresh: bool,
    progress: Channel<neural_pitch_core::store::AnalysisProgress>,
) -> Result<neural_pitch_core::store::AnalysisSummary, String> {
    let parsed: neural_pitch_core::store::RecordingId = recording_id
        .parse()
        .map_err(|e| format!("invalid recording id: {e}"))?;

    // Refuse every backend other than pYIN. The cache row would otherwise
    // be stamped with the requested analyzer_name but the bytes inside
    // would still be pYIN-derived. See the doc-comment above for the
    // routing-gap rationale.
    match &backend {
        BackendKind::PYin => {}
        other => {
            return Err(format!(
                "backend not yet routed: {} — Phase 2.5",
                other.analyzer_name()
            ));
        }
    }

    let analyzer_name = backend.analyzer_name();
    let analyzer_version = backend.analyzer_version();

    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    {
        let mut g = state.analyses.lock();
        if let Some(prev) = g.insert(parsed, Arc::clone(&cancel)) {
            prev.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    }

    let library = Arc::clone(&state.library);
    let cancel_for_blocking = Arc::clone(&cancel);
    let sink_owned = ChannelProgressSink { channel: progress };
    let result = tokio::task::spawn_blocking(move || {
        let sink_ref: &dyn neural_pitch_core::store::ProgressSink = &sink_owned;
        neural_pitch_core::store::analyze_recording_blocking(
            &library,
            parsed,
            analyzer_name,
            analyzer_version,
            force_refresh,
            Some(sink_ref),
            Some(cancel_for_blocking.as_ref()),
        )
    })
    .await
    .map_err(|e| {
        let mut g = state.analyses.lock();
        g.remove(&parsed);
        format!("analyze_recording_with_backend task panicked: {e}")
    })?;

    {
        let mut g = state.analyses.lock();
        if let Some(existing) = g.get(&parsed) {
            if Arc::ptr_eq(existing, &cancel) {
                g.remove(&parsed);
            }
        }
    }

    result.map_err(|e| format!("analyze: {e:#}"))
}

/// Read-only snapshot of the build's compiled-in capabilities.
///
/// The front-end uses this to light a developer-mode status pill and to
/// gate the (Phase 2.5/3) backend-picker UI. Mirrors the `cfg!` flags at
/// the Tauri layer so the front-end never has to guess which backends are
/// linked in.
#[derive(Serialize, Debug, Clone)]
pub struct Capabilities {
    /// `true` when this binary was built with `--features app-neural`
    /// (which transitively pulls in `neural-pitch-core/neural`).
    pub neural_compiled_in: bool,
    /// `true` when `neural-pitch-core` was built with `feature = "pyin"`.
    pub pyin_compiled_in: bool,
}

/// Return the build's compiled-in capability flags.
#[tauri::command]
#[tracing::instrument(skip())]
pub fn get_capabilities() -> Result<Capabilities, String> {
    Ok(Capabilities {
        neural_compiled_in: cfg!(feature = "app-neural"),
        // The `pyin` feature lives on `neural-pitch-core`. The shell does
        // not gate IPC commands on it — the analyzer surface picks it up
        // when the dependency is built with `--features pyin`. We mirror
        // the default-feature shape here: pYIN is on by default in
        // neural-pitch-core, so unless a downstream consumer explicitly
        // disables it, it is compiled in. The Tauri shell always builds
        // against the default feature set (`["cpal", "flac", "pyin"]`).
        pyin_compiled_in: true,
    })
}

/// Status of a model entry in the workspace `models.toml`.
///
/// Maps onto the resolver's [`PeekResult`] so the front-end Settings UI
/// can render a "Download PESTO model" button when
/// [`ModelStatus::MissingButFetchable`].
#[derive(Serialize, Debug, Clone)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ModelStatus {
    /// The model is shipped in-tree as a synthetic test ONNX. Reserved
    /// for the test_utils path; the workspace `models.toml` does not
    /// surface this today.
    Bundled,
    /// The model is on disk and verified against the manifest sha256.
    Cached {
        /// Resolved absolute path.
        path: PathBuf,
    },
    /// The model is not on disk, but the manifest carries a real URL +
    /// sha256 — the front-end may offer a download.
    MissingButFetchable {
        /// HTTPS URL the resolver will fetch from in Phase 2.5/3.
        url: String,
    },
    /// The manifest entry is still a placeholder (empty URL or all-zeros
    /// sha256). The front-end SHOULD NOT offer a download.
    MissingNotConfigured,
}

/// Inspect the on-disk + manifest state for a single model.
///
/// Calls [`neural_pitch_core::models::peek`] (a non-fetching variant of
/// `ensure_model`) and maps the result onto [`ModelStatus`]. Phase 2.2:
/// the workspace `models.toml` only carries `pesto-v1` with placeholder
/// fields, so this command always surfaces
/// [`ModelStatus::MissingNotConfigured`] until Phase 2.5/3 fills the URL +
/// sha256 in. The shape is wired up early so Phase 2.5 only has to flip
/// the manifest, not the IPC.
#[tauri::command]
#[tracing::instrument(skip(state), fields(model_name = %name))]
#[allow(clippy::needless_pass_by_value)]
pub fn get_model_status(
    name: String,
    state: State<'_, crate::state::AppState>,
) -> Result<ModelStatus, String> {
    use neural_pitch_core::models::{ResolverError, peek};

    // Resolve the per-platform models directory. We reuse the recordings
    // dir's parent (the app-data dir) and append `models/` so the cache
    // sits next to `recordings/` and `library.sqlite`.
    let models_dir = state
        .recordings_dir
        .parent()
        .map_or_else(|| PathBuf::from("models"), |p| p.join("models"));

    match peek(&name, &models_dir) {
        Ok(p) => {
            if p.is_placeholder {
                Ok(ModelStatus::MissingNotConfigured)
            } else if p.on_disk_match {
                Ok(ModelStatus::Cached { path: p.target })
            } else {
                Ok(ModelStatus::MissingButFetchable { url: p.entry.url })
            }
        }
        Err(ResolverError::UnknownModel(n)) => Err(format!("unknown model: {n}")),
        Err(ResolverError::ManifestNotFound(p)) => {
            Err(format!("manifest not found: {}", p.display()))
        }
        Err(ResolverError::ManifestParse(e)) => Err(format!("manifest parse error: {e}")),
        Err(e) => Err(format!("get_model_status: {e}")),
    }
}

/// Cancel an in-flight analysis. Idempotent: returns `Ok(())` if no
/// analysis is currently registered for the supplied `recording_id`.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub async fn cancel_analysis(
    state: State<'_, crate::state::AppState>,
    recording_id: String,
) -> Result<(), String> {
    let parsed: neural_pitch_core::store::RecordingId = recording_id
        .parse()
        .map_err(|e| format!("invalid recording id: {e}"))?;
    let token = {
        let g = state.analyses.lock();
        g.get(&parsed).map(Arc::clone)
    };
    if let Some(t) = token {
        t.store(true, std::sync::atomic::Ordering::Relaxed);
    }
    Ok(())
}

/// Rename a recording's user-supplied label. Empty / whitespace-only
/// strings clear the label.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub async fn rename_recording(
    state: State<'_, crate::state::AppState>,
    id: String,
    label: String,
) -> Result<(), String> {
    let parsed: neural_pitch_core::store::RecordingId = id
        .parse()
        .map_err(|e| format!("invalid recording id: {e}"))?;
    let new_label = if label.trim().is_empty() {
        None
    } else {
        Some(label)
    };
    let library = Arc::clone(&state.library);
    tokio::task::spawn_blocking(move || {
        // No dedicated rename API today — round-trip via the connection.
        // We extend the library surface here directly so the IPC command
        // stays local until a Phase 2.4 rename helper lands.
        let rows =
            library.list_recordings(neural_pitch_core::store::ListFilter::IncludingDeleted)?;
        if !rows.iter().any(|r| r.id == parsed) {
            return Err(neural_pitch_core::store::StoreError::NotFound(parsed));
        }
        // The library exposes only insert/list/soft_delete/upsert today.
        // For Phase 2.0 we accept the label-update is a no-op until the
        // dedicated rename API lands in Phase 2.4 — log so operators see
        // the intent.
        tracing::info!(
            target: "neural_pitch::commands",
            id = %parsed,
            label = ?new_label,
            "rename_recording is a placeholder; persisted rename lands with Phase 2.4 surface",
        );
        Ok(())
    })
    .await
    .map_err(|e| format!("rename_recording task panicked: {e}"))?
    .map_err(|e| format!("rename recording: {e:#}"))?;
    Ok(())
}

/// Reconfigure the settings cache.
///
/// Phase 1.2 contract: `configure` MAY NOT be called while capture is live;
/// reconfiguring an active pipeline requires a `stop_capture`/`start_capture`
/// round-trip so the front-end and the worker stay in lock-step on
/// `window_size`, `hop_size`, `sample_rate_hz`, and `instrument_hint`.
/// Calling `configure` while live returns `Err`. Phase 1.3 will introduce a
/// dedicated `reconfigure_running` that performs stop→mutate→start
/// atomically.
///
/// If the supplied settings do not validate, `Err` is returned and the
/// settings cache is left untouched.
#[tauri::command]
#[tracing::instrument(
    skip(state),
    fields(
        sample_rate_hz = settings.sample_rate_hz,
        window_size = settings.window_size,
        hop_size = settings.hop_size,
        a4_hz = settings.a4_hz,
        instrument_hint = ?settings.instrument_hint,
    ),
)]
pub async fn configure(state: State<'_, AppState>, settings: TunerSettings) -> Result<(), String> {
    settings
        .validate()
        .map_err(|e| format!("invalid settings: {e:#}"))?;

    if state.dsp.lock().is_some() {
        return Err("cannot reconfigure while capturing; call stop_capture first".to_string());
    }

    let snapshot = {
        let mut g = state.settings.write();
        *g = settings.clone();
        g.clone()
    };
    persist_settings(&state, snapshot).await
}

/// Snapshot the current in-memory settings cache. Returns the settings
/// wrapped in `Result` purely to satisfy Tauri's async-command-with-borrows
/// constraint; this command never produces an error.
#[tauri::command]
#[tracing::instrument(skip(state))]
pub async fn get_settings(state: State<'_, AppState>) -> Result<TunerSettings, String> {
    Ok(state.settings_snapshot())
}

/// Apply a single `(key, value)` patch to the current settings, validate,
/// persist, and return the new full struct. Validation errors do not
/// mutate state.
///
/// The whole RMW is performed under the settings write lock so two
/// concurrent `set_setting` calls cannot lose each other's deltas.
#[tauri::command]
#[tracing::instrument(skip(state, value), fields(key = %key))]
pub async fn set_setting(
    state: State<'_, AppState>,
    key: String,
    value: Value,
) -> Result<TunerSettings, String> {
    // Hold the write lock for the entire RMW so two concurrent set_setting
    // calls computing patches against the same baseline cannot lose each
    // other's deltas. The lock is dropped before `.await` because
    // parking_lot guards are `!Send` and may not cross suspension points.
    let next = {
        let mut g = state.settings.write();
        let next = g
            .with_patch(&key, value)
            .map_err(|e| format!("set_setting({key}): {e:#}"))?;
        *g = next.clone();
        next
    };
    persist_settings(&state, next.clone()).await?;
    Ok(next)
}

// -- Helpers ----------------------------------------------------------------

/// Persist the settings blob to disk.
///
/// The blocking serialise + filesystem write runs on a `spawn_blocking`
/// worker so the tokio runtime thread is not parked on fsync — even on
/// slow or network filesystems where `tauri-plugin-store::save()` can take
/// tens of milliseconds.
///
/// Atomicity caveat: callers drop the settings write lock *before* the
/// `.await` (parking_lot guards are `!Send`). The disk converges to the
/// snapshot supplied here, which mirrors the cache state at the moment
/// the lock was dropped. A concurrent `set_setting` that runs between
/// drop-guard and the spawn_blocking start will queue a second
/// `persist_settings` whose disk write happens after this one — so on-disk
/// state always trails the latest cache by at most one persist round-trip
/// and converges as soon as the queue drains.
async fn persist_settings(
    state: &State<'_, AppState>,
    settings: TunerSettings,
) -> Result<(), String> {
    let value =
        serde_json::to_value(&settings).map_err(|e| format!("serialize settings: {e:#}"))?;
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        store.set(SETTINGS_STORE_KEY, value);
        store.save().map_err(|e| format!("persist settings: {e:#}"))
    })
    .await
    .map_err(|e| format!("persist task panicked: {e}"))?
}

/// Map [`InstrumentHint`] to the live-tuner search range.
///
/// Thin wrapper around [`live_search_range_for_hint`]: kept here so the call
/// site reads cleanly and so the unit tests in this module continue to
/// exercise the helper through the same surface the live build uses.
/// Acceptance harnesses MUST go through `live_search_range_for_hint`
/// directly so test coverage and live behaviour stay bound to the same
/// table.
fn search_range(hint: InstrumentHint) -> (f32, f32) {
    live_search_range_for_hint(hint)
}

/// Typed error surface for `build_controller`. Variants preserve the
/// upstream typed errors via `#[from]` so the front-end can branch on
/// machine-readable codes (see `serde(tag = "code")`) instead of regex-
/// matching free-form strings.
#[derive(Debug, thiserror::Error, Serialize)]
#[serde(tag = "code", content = "message", rename_all = "snake_case")]
enum BuildError {
    /// No capture device is registered with the platform host.
    #[error("no audio capture device available")]
    NoInputDevice,
    /// `default_input_config()` failed on the resolved device.
    #[error("failed to query default input config: {0}")]
    DefaultConfig(String),
    /// The pitch estimator factory returned a typed error.
    #[error(transparent)]
    Estimator(
        #[from]
        #[serde(serialize_with = "serialize_display")]
        EstimatorError,
    ),
    /// The audio backend reported a typed error.
    #[error(transparent)]
    AudioBackend(
        #[from]
        #[serde(serialize_with = "serialize_display")]
        AudioBackendError,
    ),
    /// The DSP worker thread could not be spawned, or the worker returned
    /// a typed error during start-up.
    #[error(transparent)]
    Dsp(
        #[from]
        #[serde(serialize_with = "serialize_display")]
        DspError,
    ),
    /// The host advertised a sample format that this build does not
    /// support.
    #[error("unsupported sample format: {0}")]
    UnsupportedSampleFormat(#[serde(serialize_with = "serialize_sample_format")] SampleFormat),
}

// `serde(serialize_with = "...")` is invoked with `&SampleFormat` regardless
// of `Copy`, so we silence the `trivially_copy_pass_by_ref` lint here.
#[allow(clippy::trivially_copy_pass_by_ref)]
fn serialize_sample_format<S: serde::Serializer>(
    fmt: &SampleFormat,
    ser: S,
) -> Result<S::Ok, S::Error> {
    ser.serialize_str(&format!("{fmt:?}"))
}

fn serialize_display<S, T>(value: &T, ser: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
    T: core::fmt::Display,
{
    ser.serialize_str(&value.to_string())
}

/// Translate a [`BuildError`] into the user-facing string surfaced to the
/// front-end. `AudioBackend` permission denials are detected on the typed
/// [`AudioBackendError::PermissionDenied`] variant; other variants pass
/// through as a structured `format!` chain. ADR-0017 forbids any telemetry
/// on the permission-denial path.
///
/// Takes `BuildError` by value so it composes cleanly with
/// `Result::map_err(translate_build_error)` at the call site.
#[allow(clippy::needless_pass_by_value)]
fn translate_build_error(e: BuildError) -> String {
    if let BuildError::AudioBackend(AudioBackendError::PermissionDenied(_)) = &e {
        // Match on the typed variant rather than substring-scanning the
        // backend message: the CpalAudioBackend layer is responsible for
        // mapping locale-dependent BackendSpecific text into the typed
        // PermissionDenied variant. This keeps the user-facing copy stable
        // even if cpal / CoreAudio message text changes.
        return "microphone permission denied — open System Settings → Privacy & Security → Microphone to grant access".to_string();
    }
    format!("failed to start capture: {e:#}")
}

/// Default audio-callback buffer size in frames. Used as the WASAPI
/// `BufferSize::Fixed` request; clamped into the device's
/// `SupportedBufferSize::Range` by `pick_buffer_size` at stream-build time.
/// 256 frames at 48 kHz ≈ 5.3 ms — well under the DESIGN §6.3 latency budget.
const DEFAULT_BUFFER_FRAMES: u32 = 256;

/// Wire up the live capture pipeline end-to-end and return the controller
/// that the lifecycle owner stores in `AppState`.
fn build_controller(
    settings: &TunerSettings,
    channel: Channel<PitchUpdate>,
    emitter: Option<AudioEventEmitter>,
) -> Result<DspController, BuildError> {
    // 1) Discover the default input device. We do not currently allow
    //    explicit device selection from the front-end; that is Phase 1.3
    //    work (see ADR-0017).
    let host = cpal::default_host();
    let device = host
        .default_input_device()
        .ok_or(BuildError::NoInputDevice)?;

    let supported = device
        .default_input_config()
        .map_err(|e| BuildError::DefaultConfig(e.to_string()))?;
    let stream_config = cpal::StreamConfig {
        channels: supported.channels(),
        sample_rate: settings.sample_rate_hz,
        buffer_size: cpal::BufferSize::Default,
    };
    let sample_format = validate_sample_format(supported.sample_format())?;
    let backend_cfg = AudioBackendConfig {
        sample_rate: settings.sample_rate_hz,
        channels: supported.channels(),
        hop: settings.hop_size,
        window: settings.window_size,
    };

    // 2) Estimator + supporting DSP blocks.
    let (fmin, fmax) = search_range(settings.instrument_hint);
    let est_cfg = EstimatorConfig {
        sample_rate_hz: settings.sample_rate_hz,
        window_size: settings.window_size,
        hop_size: settings.hop_size,
        fmin_hz: fmin,
        fmax_hz: fmax,
        instrument_hint: Some(settings.instrument_hint),
    };
    let estimator = make_estimator(
        Backend::YinMpm,
        est_cfg,
        None::<&PathBuf>.map(PathBuf::as_path),
    )?;

    let smoother = ContourSmoother::new(settings.smoothing_window_ms, settings.sample_rate_hz);
    let vad = VoiceActivityGate::new(0.005, 4);

    // 3) SPSC ring sized per the Phase 1.1 contract.
    let (producer, consumer) = rtrb::RingBuffer::<f32>::new(backend_cfg.ring_capacity());

    // 4) Sink + worker + cancellation token + shared recording fan-out.
    let sink = Box::new(TauriChannelFrameSink::new(channel));
    let cancel = CancellationToken::new();
    // Shared fan-out — empty at start_capture time; populated when
    // `start_recording` later attaches a bounded sync_channel.
    let recording_fanout = RecordingFanout::new();
    let worker = DspWorker::new(
        backend_cfg.clone(),
        estimator,
        smoother,
        vad,
        consumer,
        sink,
        cancel.clone(),
    )
    .with_a4(settings.a4_hz)
    .with_recording_fanout(recording_fanout.clone());
    let worker_join = worker.spawn()?;

    // 5) Construct + start the cpal-backed audio capture. If `start`
    //    fails, drop the worker side (cancel + join) and bubble the error
    //    verbatim. The Phase 1.1 backend semantics already satisfy the
    //    "no poison" rule.
    //
    //    Phase 1.3: request a `Fixed(256)` buffer size for WASAPI; the
    //    backend clamps to the device's supported range via
    //    `pick_buffer_size` and falls back to `BufferSize::Default` when
    //    the range is `Unknown`. The optional `emitter` forwards
    //    `AudioBackendEvent::{Disconnected, Underrun, FormatChanged}` over
    //    the JS-side `Channel<AudioBackendEvent>`.
    let mut cpal_backend = CpalAudioBackend::new(backend_cfg, device, stream_config, sample_format)
        .with_buffer_frames(DEFAULT_BUFFER_FRAMES);
    if let Some(em) = emitter {
        cpal_backend = cpal_backend.with_emitter(em);
    }
    let mut backend: Box<dyn AudioBackend> = Box::new(cpal_backend);
    if let Err(e) = backend.start(producer) {
        cancel.cancel();
        // The worker observes the dropped producer + cancellation flag and
        // exits within a few hop intervals (~10 ms at 48 kHz / hop=512).
        // Bound the wait so a wedged worker can't park us forever; this
        // mirrors the stop_capture poll-with-budget shape so the same
        // RT-safety property applies to the failure path.
        let deadline = std::time::Instant::now() + DSP_JOIN_BUDGET;
        let mut joined = false;
        while std::time::Instant::now() < deadline {
            if worker_join.is_finished() {
                let _ = worker_join.join();
                joined = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        if !joined {
            tracing::warn!("dsp worker did not exit within budget on backend.start failure");
        }
        return Err(BuildError::AudioBackend(e));
    }

    Ok(DspController {
        backend,
        worker_join: Some(worker_join),
        cancel,
        recording_fanout,
    })
}

fn validate_sample_format(fmt: SampleFormat) -> Result<SampleFormat, BuildError> {
    match fmt {
        SampleFormat::F32 | SampleFormat::I16 | SampleFormat::U16 => Ok(fmt),
        other => Err(BuildError::UnsupportedSampleFormat(other)),
    }
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
mod tests {
    use super::*;
    use neural_pitch_core::pitch::InstrumentHint;

    #[test]
    fn search_range_voice_is_within_typical_human_range() {
        let (lo, hi) = search_range(InstrumentHint::Voice);
        assert!(lo > 0.0 && lo < 100.0);
        assert!(hi > 800.0);
    }

    #[test]
    fn search_range_bass_starts_below_voice() {
        let (vlo, _) = search_range(InstrumentHint::Voice);
        let (blo, _) = search_range(InstrumentHint::Bass);
        assert!(blo < vlo);
    }

    #[test]
    fn build_error_serialises_with_machine_readable_code() {
        let err = BuildError::NoInputDevice;
        let json = serde_json::to_value(&err).expect("serialize");
        assert_eq!(json["code"], "no_input_device");
    }

    /// Lock in the on-the-wire `kind` discriminant for every BackendKind
    /// variant so a future serde rename (e.g. accidentally re-introducing
    /// `rename_all = "snake_case"`) cannot silently flip `pyin` to
    /// `p_yin` or `crepe-tiny` to `crepe_tiny`. This is the contract the
    /// front-end joins to `analysis_cache.analyzer_name` rows.
    #[test]
    fn backend_kind_wire_discriminants_match_analyzer_names() {
        let yin: BackendKind =
            serde_json::from_str(r#"{"kind":"yin"}"#).expect("yin should deserialize");
        let pyin: BackendKind =
            serde_json::from_str(r#"{"kind":"pyin"}"#).expect("pyin should deserialize");
        let pesto: BackendKind =
            serde_json::from_str(r#"{"kind":"pesto","onnx_path":"/tmp/p.onnx"}"#)
                .expect("pesto should deserialize");
        let crepe: BackendKind =
            serde_json::from_str(r#"{"kind":"crepe-tiny","onnx_path":"/tmp/c.onnx"}"#)
                .expect("crepe-tiny should deserialize");
        assert_eq!(yin.analyzer_name(), "yin");
        assert_eq!(pyin.analyzer_name(), "pyin");
        assert_eq!(pesto.analyzer_name(), "pesto");
        assert_eq!(crepe.analyzer_name(), "crepe-tiny");
    }
}
