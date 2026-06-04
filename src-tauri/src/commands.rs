//! Tauri command surface for the NeuralPitch shell.
//!
//! All commands return `Result<T, String>` per ADR-0015 — errors are
//! formatted with `format!("{e:#}")` so the front-end gets the full
//! `anyhow`-style chain. Validation failures do not mutate state.

use std::path::PathBuf;
use std::time::Duration;

use cpal::SampleFormat;
use cpal::traits::{DeviceTrait, HostTrait};
use neural_pitch_core::audio::backend::{AudioBackend, AudioBackendConfig, AudioBackendError};
use neural_pitch_core::audio::cpal_backend::CpalAudioBackend;
use neural_pitch_core::pipeline::{DspError, DspWorker, PitchUpdate};
use neural_pitch_core::pitch::factory::{Backend, make_estimator};
use neural_pitch_core::pitch::{EstimatorConfig, EstimatorError, InstrumentHint};
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
/// frames through `channel`.
///
/// Failure semantics — strictly atomic with respect to disk + in-memory
/// state. The settings cache and the on-disk store are mutated only after
/// `build_controller` succeeds; any earlier validation, "already capturing",
/// or backend-construction failure leaves the caller's prior settings
/// intact.
#[tauri::command]
#[tracing::instrument(
    skip(state, channel),
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

    let controller = build_controller(&settings, channel)
        .map_err(|e| format!("failed to start capture: {e:#}"))?;

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
        // already created a second backend; tear it down before bailing so
        // we don't leak a cpal stream.
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

/// Map [`InstrumentHint`] to a default search range. Phase 1.3 will replace
/// this with a richer auto-prior; for now the bounds are conservative
/// supersets of the relevant fundamental ranges.
fn search_range(hint: InstrumentHint) -> (f32, f32) {
    match hint {
        InstrumentHint::Voice => (60.0, 1100.0),
        InstrumentHint::Guitar => (70.0, 1400.0),
        InstrumentHint::Bass => (30.0, 600.0),
        InstrumentHint::Piano => (25.0, 4500.0),
        InstrumentHint::Violin => (180.0, 3600.0),
        InstrumentHint::Generic => (50.0, 1500.0),
    }
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

/// Wire up the live capture pipeline end-to-end and return the controller
/// that the lifecycle owner stores in `AppState`.
fn build_controller(
    settings: &TunerSettings,
    channel: Channel<PitchUpdate>,
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

    // 4) Sink + worker + cancellation token.
    let sink = Box::new(TauriChannelFrameSink::new(channel));
    let cancel = CancellationToken::new();
    let worker = DspWorker::new(
        backend_cfg.clone(),
        estimator,
        smoother,
        vad,
        consumer,
        sink,
        cancel.clone(),
    )
    .with_a4(settings.a4_hz);
    let worker_join = worker.spawn()?;

    // 5) Construct + start the cpal-backed audio capture. If `start`
    //    fails, drop the worker side (cancel + join) and bubble the error
    //    verbatim. The Phase 1.1 backend semantics already satisfy the
    //    "no poison" rule.
    let mut backend: Box<dyn AudioBackend> = Box::new(CpalAudioBackend::new(
        backend_cfg,
        device,
        stream_config,
        sample_format,
    ));
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
}
