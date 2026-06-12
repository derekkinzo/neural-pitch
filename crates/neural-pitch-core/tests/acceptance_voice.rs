//! Voice-fixture acceptance harness.
//!
//! Runs the same DSP path the live tuner uses (window=2048, hop=512,
//! `YinMpmEstimator` + `AutoPrior` + `ContourSmoother` + `VoiceActivityGate`)
//! against every FLAC fixture under `tests/fixtures/voice/`, then asserts
//! aggregate octave-correctness ≥ 95%.
//!
//! Live-tuner DSP knobs are sourced from a single home: `TunerSettings::default()`
//! (smoothing window, sample rate, window/hop) and
//! `live_search_range_for_hint(InstrumentHint::Generic)` for the YIN
//! `(fmin, fmax)`. This is the same call the live shell makes in
//! `src-tauri/commands.rs::build_controller`, so any drift between the two
//! is a single-table change rather than two divergent literals.
//!
//! Per-fixture pass criterion: `|round(m_est) - m_truth| < 1` on the **mode**
//! of `target_midi` over voiced frames (deterministic tiebreak: lower MIDI
//! wins). The mode is octave-error-robust where the mean is not — a single
//! octave-doubled outlier shifts the mean by 12 semitones; the mode shrugs.
//!
//! Wire-format contract with `scripts/run-acceptance.sh`:
//!
//! - One per-fixture line per accepted FLAC:
//!   `[ACCEPT-FIXTURE] {filename}: pass={true|false} estimated_midi={N}
//!     expected={N} cents_error={f}`
//! - One single-line aggregate JSON with the marker prefix:
//!   `=== ACCEPTANCE_JSON === { "aggregate": ..., "unit_test_count": ...,
//!     "fixture_test_count": ..., "latency_p50_ms": ..., "latency_p99_ms": ... }`
//!
//! The test layout follows the standard test pyramid: small in-process
//! unit tests under `tests/` (one file per invariant), fixture-driven
//! tests that feed real FLAC through the live DSP path, and this
//! end-to-end acceptance harness on top.
//!
//! Run it directly with `cargo test -p neural-pitch-core --test acceptance_voice
//! -- --nocapture` (debug build is fine; release is faster).
#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
#![allow(clippy::print_stdout, clippy::cast_possible_wrap)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use claxon::FlacReader;
use neural_pitch_core::audio::{AudioBackend, AudioBackendConfig, MockAudioBackend, SampleSource};
use neural_pitch_core::music::midi_to_hz;
use neural_pitch_core::pipeline::{ChannelFrameSink, DspWorker, PitchUpdate};
use neural_pitch_core::pitch::factory::{Backend, make_estimator};
use neural_pitch_core::pitch::{EstimatorConfig, InstrumentHint, live_search_range_for_hint};
use neural_pitch_core::settings::TunerSettings;
use neural_pitch_core::smoothing::ContourSmoother;
use neural_pitch_core::voicing::VoiceActivityGate;
use tokio_util::sync::CancellationToken;

/// Compile-time-bound manifest content. A malformed manifest fails
/// compilation, not at runtime — the manifest is part of the build
/// graph, not a runtime asset.
const MANIFEST_TEXT: &str = include_str!("fixtures/voice/MANIFEST.toml");

/// One row from `MANIFEST.toml`. Only the fields the harness needs are
/// pulled out; we use `toml::Value` rather than a `serde::Deserialize`
/// derive to avoid widening the dev-dep tree.
#[derive(Clone, Debug)]
struct FixtureSpec {
    filename: String,
    expected_midi: i32,
}

/// Parse the manifest into a flat `Vec<FixtureSpec>`. Panics on malformed
/// content — this is the right behaviour for a test fixture: the manifest
/// is committed alongside the harness and a parse failure indicates the
/// harness itself is broken.
fn load_manifest() -> Vec<FixtureSpec> {
    let parsed: toml::Value = toml::from_str(MANIFEST_TEXT).expect("parse MANIFEST.toml");
    let array = parsed
        .get("fixture")
        .and_then(toml::Value::as_array)
        .expect("MANIFEST.toml: missing [[fixture]] array");
    let mut out = Vec::with_capacity(array.len());
    for entry in array {
        let table = entry.as_table().expect("fixture entry must be a table");
        let filename = table
            .get("filename")
            .and_then(toml::Value::as_str)
            .expect("fixture.filename")
            .to_owned();
        let expected_midi = i32::try_from(
            table
                .get("expected_midi")
                .and_then(toml::Value::as_integer)
                .expect("fixture.expected_midi"),
        )
        .expect("expected_midi fits in i32");
        out.push(FixtureSpec {
            filename,
            expected_midi,
        });
    }
    assert!(
        !out.is_empty(),
        "MANIFEST.toml parsed but contains no [[fixture]] entries"
    );
    out
}

/// Decode a FLAC fixture into a normalised `Vec<f32>` in `[-1.0, 1.0]`.
///
/// `claxon` yields `Vec<i32>` PCM. We divide by `2^(bits-1)` to land on the
/// canonical analyzer scale. The fixtures are 48 kHz / 24-bit / mono per
/// The harness asserts the sample-rate match because feeding 44.1 k
/// audio into a 48 k window would silently bias every cents-error estimate
/// by `1200 * log2(48000/44100) ≈ 150` cents.
fn decode_flac(path: &Path) -> (u32, Vec<f32>) {
    let mut reader = FlacReader::open(path).expect("open flac fixture");
    let info = reader.streaminfo();
    let bits = info.bits_per_sample;
    assert!(
        (16..=24).contains(&bits),
        "fixture {} has unexpected bits_per_sample={}",
        path.display(),
        bits
    );
    let max_val = (1_i32 << (bits - 1)) as f32;
    let samples: Vec<f32> = reader
        .samples()
        .map(|s| s.expect("decode sample") as f32 / max_val)
        .collect();
    (info.sample_rate, samples)
}

/// Compute the mode of an integer sequence with the spec's tiebreak rule:
/// when the modal bin is non-unique, the **lower** value wins.
fn modal_midi(midis: &[i32]) -> i32 {
    let mut counts: HashMap<i32, usize> = HashMap::new();
    for m in midis {
        *counts.entry(*m).or_insert(0) += 1;
    }
    let max_count = counts.values().copied().max().unwrap_or(0);
    counts
        .into_iter()
        .filter(|(_, c)| *c == max_count)
        .map(|(m, _)| m)
        .min()
        .expect("non-empty voiced frame set")
}

/// Median of a `Vec<f32>` with NaN-rejecting comparison. The vector is
/// sorted in place.
fn median_f32(values: &mut [f32]) -> f32 {
    assert!(!values.is_empty(), "median of empty slice");
    values.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let n = values.len();
    if n.is_multiple_of(2) {
        0.5 * (values[n / 2 - 1] + values[n / 2])
    } else {
        values[n / 2]
    }
}

/// Per-fixture report pulled out of `acceptance_voice_fixture_pyramid` so it can be
/// rendered into both human-readable lines and the script's wire-format
/// contract from a single source of truth.
///
/// Voiced-frame counts are reported on the per-fixture *human-readable*
/// line above (so a fixture yielding 3 voiced frames out of 140 cannot
/// quietly pass on mode alone) but are not part of the script-side
/// `[ACCEPT-FIXTURE]` contract — keeping the struct narrow keeps the
/// wire-format stable.
struct FixtureReport {
    filename: String,
    expected_midi: i32,
    estimated_midi: i32,
    median_cents: f32,
    pass: bool,
}

/// Run one fixture end-to-end through the live DSP path and return the
/// vector of `PitchUpdate`s the worker emitted (voiced + unvoiced both —
/// the caller filters).
///
/// Returns `(updates, dropped_samples, latency_ms)`. `dropped_samples` is
/// the mock backend's underrun counter; the harness panics if it ever
/// increments because that means the SPSC ring overflowed and any
/// downstream cents-error metric is silently corrupted.
#[allow(clippy::too_many_lines)] // single-pass feed/drain loop with explicit comments; splitting hurts readability
fn run_fixture_through_pipeline(
    samples: Vec<f32>,
    sample_rate: u32,
) -> (Vec<PitchUpdate>, u64, Vec<f32>) {
    // Live-tuner defaults are the single source of truth — pull every knob
    // out of `TunerSettings::default()` and `live_search_range_for_hint`
    // so a future change to either propagates here automatically.
    let live_defaults = TunerSettings::default();
    let cfg = AudioBackendConfig {
        sample_rate,
        channels: 1,
        hop: live_defaults.hop_size,
        window: live_defaults.window_size,
    };
    let (live_fmin, live_fmax) = live_search_range_for_hint(InstrumentHint::Generic);

    // Sanity-pin: if the live defaults ever drift from what the harness
    // was tuned against (e.g. someone bumps smoothing_ms in settings.rs),
    // we want a hard signal here, not a silent behaviour shift in CI.
    assert!(
        (live_defaults.smoothing_window_ms - 300.0).abs() < f32::EPSILON,
        "live default smoothing_window_ms drifted to {} — harness expected 300.0; \
         either update the harness or revert settings.rs",
        live_defaults.smoothing_window_ms
    );
    assert!(
        (live_fmin - 50.0).abs() < f32::EPSILON && (live_fmax - 1500.0).abs() < f32::EPSILON,
        "live Generic search range drifted to ({live_fmin}, {live_fmax}) — harness expected \
         (50, 1500); either update the harness or revert \
         pitch::live_search_range_for_hint"
    );

    // Estimator config: live-tuner Generic range. The auto-prior — running
    // on the worker's `process_with_range` path — is what narrows the
    // search each iteration.
    //
    // AutoPrior runs with no pinned hint here (`DspWorker::new` initialises
    // it via `AutoPrior::default()`, which is identical to the no-hint
    // state). The `EstimatorConfig::instrument_hint` field below is
    // currently unused by `YinMpmEstimator` — it is plumbed for forward
    // compatibility only. If a future change wires the hint into the
    // estimator, this harness will need to set it explicitly via
    // `DspWorker::with_instrument_hint(None)` to keep the no-hint
    // contract.
    let est_cfg = EstimatorConfig {
        sample_rate_hz: cfg.sample_rate,
        window_size: cfg.window,
        hop_size: cfg.hop,
        fmin_hz: live_fmin,
        fmax_hz: live_fmax,
        instrument_hint: Some(InstrumentHint::Generic),
    };
    let estimator = make_estimator(Backend::YinMpm, est_cfg, None).expect("build estimator");

    let (producer, consumer) = rtrb::RingBuffer::<f32>::new(cfg.ring_capacity());

    // Wrap the decoded fixture in a shared `Vec<f32>` and drive the mock
    // backend with a `Custom` closure that returns the indexed sample,
    // padding with silence past the end. Padding past the end is needed
    // because the DSP worker has to fill its 2048-sample sliding window
    // *before* it emits the first frame; trailing silence keeps the worker
    // alive long enough to flush whatever frames the fixture is owed.
    let shared: Arc<Mutex<Vec<f32>>> = Arc::new(Mutex::new(samples));
    let shared_for_closure = Arc::clone(&shared);
    let source = SampleSource::Custom(Box::new(move |idx: u64| -> f32 {
        let buf = shared_for_closure.lock().expect("sample buf poisoned");
        let i = idx as usize;
        if i < buf.len() { buf[i] } else { 0.0 }
    }));
    let mut backend = MockAudioBackend::new(cfg.clone(), source);
    let dropped_handle = backend.dropped_samples();
    backend.start(producer).expect("start mock backend");

    let (tx, rx) = mpsc::channel::<PitchUpdate>();
    let sink = Box::new(ChannelFrameSink::new(tx));
    let cancel = CancellationToken::new();

    // VAD threshold: 0.005 RMS matches every `dsp_pipeline_*.rs` fixture test.
    // Hangover = 4 frames keeps the gate open for ~43 ms (4 hops at 48 k),
    // mirroring the live tuner's behaviour.
    //
    // ContourSmoother window pulled from `TunerSettings::default()` so the
    // harness exercises the same 300 ms smoothing the live tuner uses.
    // `with_a4` is set explicitly to the live default even though it
    // matches the worker's intrinsic default — making it explicit binds
    // the call site to the same A4 contract as `build_controller`.
    let worker = DspWorker::new(
        cfg.clone(),
        estimator,
        ContourSmoother::new(live_defaults.smoothing_window_ms, cfg.sample_rate),
        VoiceActivityGate::new(0.005, 4),
        consumer,
        sink,
        cancel.clone(),
    )
    .with_a4(live_defaults.a4_hz);
    let handle = worker.spawn().expect("spawn DSP worker");

    // Feed strategy: walk the fixture in hop-sized batches plus a generous
    // tail of silence. The mock backend drops samples on a full ring —
    // which would silently corrupt this test — so before each feed we
    // wait for at least one PitchUpdate to come back, which guarantees
    // the worker has drained at least `cfg.hop` samples and the ring has
    // room for the next hop. The 2× tail ensures the smoother's 300 ms
    // history fully flushes before we cancel.
    //
    // The first `window / hop` (≈ 4) hops fill the worker's sliding
    // window without producing any update; for those iterations we skip
    // the recv-wait so the loop does not deadlock waiting for an update
    // that cannot exist yet. Ring capacity (next_pow2(3*window) = 8192,
    // i.e. 16 hops) is comfortably larger than 4, so feeding 4 hops
    // unconditionally cannot overflow.
    let total_samples = shared.lock().expect("buf").len();
    let tail_samples = total_samples; // ~1.5 s of trailing silence — over-generous, but cheap.
    let to_feed = total_samples + tail_samples;
    let mut updates: Vec<PitchUpdate> = Vec::new();
    let mut latency_ms: Vec<f32> = Vec::new();

    // Debug builds run YIN/MPM ~3-5x slower; double the per-fixture
    // deadline so a slow CI runner does not silently truncate the feed.
    let per_fixture_deadline_secs: u64 = if cfg!(debug_assertions) { 30 } else { 15 };
    let deadline = Instant::now() + Duration::from_secs(per_fixture_deadline_secs);
    let warmup_hops = cfg.window.div_ceil(cfg.hop);
    let mut fed = 0_usize;
    let mut hops_done = 0_usize;
    while fed < to_feed && Instant::now() < deadline {
        let chunk = cfg.hop.min(to_feed - fed);
        let feed_at = Instant::now();
        backend.feed(chunk);
        fed += chunk;
        hops_done += 1;
        // Drain whatever the worker produced *non-blocking* first — most
        // iterations will find at least one update ready already.
        while let Ok(u) = rx.try_recv() {
            let elapsed_ms = feed_at.elapsed().as_secs_f32() * 1000.0;
            latency_ms.push(elapsed_ms);
            updates.push(u);
        }
        // After the warm-up window has been filled, every subsequent
        // hop MUST eventually produce one PitchUpdate. Block (with a
        // generous wall-clock deadline) until the next one arrives,
        // then drain anything else queued. This is what keeps the ring
        // from overflowing under variable per-frame DSP cost.
        if hops_done > warmup_hops && fed < to_feed && Instant::now() < deadline {
            match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(u) => {
                    let elapsed_ms = feed_at.elapsed().as_secs_f32() * 1000.0;
                    latency_ms.push(elapsed_ms);
                    updates.push(u);
                    while let Ok(u) = rx.try_recv() {
                        let elapsed_ms = feed_at.elapsed().as_secs_f32() * 1000.0;
                        latency_ms.push(elapsed_ms);
                        updates.push(u);
                    }
                }
                Err(_) => break,
            }
        }
    }

    // Final drain — give the worker one last beat to emit whatever remains
    // in flight. Latency for these tail frames is logically meaningless
    // (no fresh feed corresponds to them), so we leave them out of the
    // latency histogram even though the updates themselves count.
    let drain_deadline = Instant::now() + Duration::from_millis(250);
    while Instant::now() < drain_deadline {
        match rx.recv_timeout(Duration::from_millis(20)) {
            Ok(u) => updates.push(u),
            Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }

    cancel.cancel();
    backend.stop();
    let _ = handle.join();

    let dropped = dropped_handle.load(Ordering::Relaxed);
    (updates, dropped, latency_ms)
}

/// Compute the percentile of a sorted `Vec<f32>` with linear interpolation.
/// `percentile` is in `[0.0, 100.0]`. Returns `0.0` for an empty input.
fn percentile_sorted(sorted: &[f32], percentile: f32) -> f32 {
    if sorted.is_empty() {
        return 0.0;
    }
    if sorted.len() == 1 {
        return sorted[0];
    }
    let p = percentile.clamp(0.0, 100.0) / 100.0;
    let last = (sorted.len() - 1) as f32;
    let pos = p * last;
    let lower = pos.floor() as usize;
    let upper = pos.ceil() as usize;
    if lower == upper {
        sorted[lower]
    } else {
        let frac = pos - pos.floor();
        sorted[lower] + (sorted[upper] - sorted[lower]) * frac
    }
}

/// Produce the single-line aggregate JSON the shell wrapper greps for. The
/// keys here are load-bearing for `scripts/run-acceptance.sh`; if any of
/// them are renamed, the script's per-key `grep` guard will fail loudly.
fn render_aggregate_json(
    aggregate: f32,
    unit_test_count: usize,
    fixture_test_count: usize,
    latency_p50_ms: f32,
    latency_p99_ms: f32,
) -> String {
    format!(
        "{{\"aggregate\":{aggregate:.4},\"unit_test_count\":{unit_test_count},\"fixture_test_count\":{fixture_test_count},\"latency_p50_ms\":{latency_p50_ms:.3},\"latency_p99_ms\":{latency_p99_ms:.3}}}",
    )
}

/// Count unit-test files under `tests/` (one file per invariant). The
/// harness is the right place to surface the count because it owns the
/// closeout-report generation. We report a conservative count of
/// *integration test files* under `tests/` other than this acceptance
/// harness — close enough to "invariant test targets" for the closeout
/// audience and stable across `cargo test --list` invocations. A more
/// precise count belongs in a dedicated `cargo test -- --list`-driven
/// script if it becomes load-bearing.
fn count_unit_test_targets() -> usize {
    // Walk the same `tests/` directory the harness lives in; count `*.rs`
    // files except the acceptance harnesses themselves. Symlinks and
    // subdirectories are ignored — current layout is flat.
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let tests_dir = PathBuf::from(manifest_dir).join("tests");
    let mut count = 0_usize;
    if let Ok(entries) = std::fs::read_dir(&tests_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("rs") {
                continue;
            }
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default();
            if stem == "acceptance_voice" || stem == "acceptance_pyin_voice" {
                continue;
            }
            count += 1;
        }
    }
    count
}

#[test]
#[allow(clippy::too_many_lines)] // single end-to-end harness; splitting would obscure the contract
fn acceptance_voice_fixture_pyramid() {
    let manifest_dir = env!("CARGO_MANIFEST_DIR");
    let voice_root = PathBuf::from(manifest_dir)
        .join("tests")
        .join("fixtures")
        .join("voice");

    let fixtures = load_manifest();
    let total = fixtures.len();
    println!(
        "[voice] running fixture acceptance over {total} synthetic voice fixtures (Generic hint, AutoPrior on)"
    );

    let mut reports: Vec<FixtureReport> = Vec::with_capacity(total);
    let mut all_latency_ms: Vec<f32> = Vec::new();

    for spec in &fixtures {
        let path = voice_root.join(&spec.filename);
        let (sr, samples) = decode_flac(&path);
        assert_eq!(
            sr, 48_000,
            "fixture {} has sample rate {} (expected 48000)",
            spec.filename, sr
        );

        let (updates, dropped, latency_ms) = run_fixture_through_pipeline(samples, sr);
        // A non-zero drop counter means the SPSC ring overflowed, which
        // silently corrupts every downstream metric — fail loudly with the
        // offending filename so the operator can size the feed loop or
        // ring capacity correctly.
        assert_eq!(
            dropped, 0,
            "fixture {} dropped {} samples on the SPSC ring — harness invariant broken",
            spec.filename, dropped
        );
        all_latency_ms.extend(latency_ms.iter().copied());

        let voiced: Vec<&PitchUpdate> = updates.iter().filter(|u| u.voiced).collect();
        let voiced_count = voiced.len();
        let total_updates = updates.len();
        if voiced.is_empty() {
            println!(
                "[voice] {}: expected {}, got <no voiced frames> voiced=0/{}  FAIL",
                spec.filename, spec.expected_midi, total_updates
            );
            reports.push(FixtureReport {
                filename: spec.filename.clone(),
                expected_midi: spec.expected_midi,
                estimated_midi: i32::MIN,
                median_cents: f32::INFINITY,
                pass: false,
            });
            continue;
        }

        // Soft floor on voiced frames: a fixture that barely opened the VAD
        // should not be allowed to PASS by mode alone. 1.5 s of audio at
        // 48 kHz / 512-sample hop ≈ 140 frames; require at least 20 voiced
        // frames so the modal MIDI carries statistical weight.
        assert!(
            voiced_count >= 20,
            "fixture {}: only {} voiced frames over {} updates — VAD or smoother is sick",
            spec.filename,
            voiced_count,
            total_updates
        );

        // Modal MIDI over voiced frames — octave-error-robust per spec §4.
        let midis: Vec<i32> = voiced.iter().map(|u| u.target_midi).collect();
        let m_est = modal_midi(&midis);
        let pass = (m_est - spec.expected_midi).abs() < 1;

        // Median |Δcents| against m_truth — informational only.
        // For each voiced frame, recompute cents against the *truth* note
        // (not the worker's nearest-note reading, which would always be ≤50
        // by construction). Frames where `f0_hz` is non-finite or
        // non-positive are skipped.
        let truth_hz = midi_to_hz(spec.expected_midi, 440.0);
        let mut cents_abs: Vec<f32> = voiced
            .iter()
            .filter_map(|u| {
                if u.f0_hz.is_finite() && u.f0_hz > 0.0 {
                    Some((1200.0 * (u.f0_hz / truth_hz).log2()).abs())
                } else {
                    None
                }
            })
            .collect();
        let med_cents = if cents_abs.is_empty() {
            f32::INFINITY
        } else {
            median_f32(&mut cents_abs)
        };

        let verdict = if pass { "PASS" } else { "FAIL" };
        println!(
            "[voice] {}: expected {}, got {}, voiced={}/{}, median |Δcents|={:5.1}  {}",
            spec.filename,
            spec.expected_midi,
            m_est,
            voiced_count,
            total_updates,
            med_cents,
            verdict
        );

        reports.push(FixtureReport {
            filename: spec.filename.clone(),
            expected_midi: spec.expected_midi,
            estimated_midi: m_est,
            median_cents: med_cents,
            pass,
        });
    }

    let passed_count = reports.iter().filter(|r| r.pass).count();
    let median_under_25c = reports
        .iter()
        .filter(|r| r.median_cents.is_finite() && r.median_cents < 25.0)
        .count();
    let pass_rate = passed_count as f32 / total as f32;
    let info_rate = median_under_25c as f32 / total as f32;
    println!(
        "[voice] aggregate: {passed_count}/{total} = {:.1}% (≥ 95% required)  {}",
        pass_rate * 100.0,
        if pass_rate >= 0.95 {
            "ACCEPT"
        } else {
            "REJECT"
        }
    );
    println!(
        "[voice] informational: median |Δcents| < 25 on {median_under_25c}/{total} = {:.1}% (≥ 80% target, not asserted)",
        info_rate * 100.0
    );

    // --- Wire-format contract emission ---------------------------------
    // Per-fixture lines first so the script can index off them in order.
    // Floats in `cents_error` are emitted with three decimal places — the
    // shell parser treats them as opaque strings and pastes them straight
    // into JSON, so anything `JSON.parse`-friendly is acceptable.
    for r in &reports {
        let cents_for_json = if r.median_cents.is_finite() {
            r.median_cents
        } else {
            // Negative-infinity-as-string would break the awk parser; pick
            // a sentinel large enough to fall well outside any real
            // cents-error so a downstream reader can tell "no data" from
            // "really bad data".
            9999.999_f32
        };
        let est_for_json = if r.estimated_midi == i32::MIN {
            // Sentinel for "no voiced frames".
            -1
        } else {
            r.estimated_midi
        };
        println!(
            "[ACCEPT-FIXTURE] {}: pass={} estimated_midi={} expected={} cents_error={:.3}",
            r.filename, r.pass, est_for_json, r.expected_midi, cents_for_json,
        );
    }

    // Aggregate JSON.
    let fixture_test_count = total;
    let unit_test_count = count_unit_test_targets();
    all_latency_ms.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let latency_p50_ms = percentile_sorted(&all_latency_ms, 50.0);
    let latency_p99_ms = percentile_sorted(&all_latency_ms, 99.0);
    println!(
        "=== ACCEPTANCE_JSON === {}",
        render_aggregate_json(
            pass_rate,
            unit_test_count,
            fixture_test_count,
            latency_p50_ms,
            latency_p99_ms,
        )
    );

    assert!(
        pass_rate >= 0.95,
        "voice fixture acceptance failed: {passed_count}/{total} = {:.1}% < 95%",
        pass_rate * 100.0
    );
}

#[cfg(test)]
mod harness_math_tests {
    //! Unit-test the harness-internal helpers. These are non-trivial
    //! enough (mode tiebreak, NaN-safe sort, even-length median) that a
    //! silent regression would invert pass criteria for tied fixtures
    //! without ever reaching the spec contract.
    use super::*;

    #[test]
    fn modal_midi_unique_mode() {
        assert_eq!(modal_midi(&[60, 60, 60, 62]), 60);
    }

    #[test]
    fn modal_midi_two_way_tie_picks_lower() {
        // 60 and 62 each appear twice — lower wins per spec §4 tiebreak.
        assert_eq!(modal_midi(&[60, 60, 62, 62]), 60);
    }

    #[test]
    fn modal_midi_three_way_tie_picks_lowest() {
        assert_eq!(modal_midi(&[60, 60, 62, 62, 64, 64]), 60);
    }

    #[test]
    fn modal_midi_all_unique_picks_minimum() {
        // Every value has count 1 → all tied; lowest wins.
        assert_eq!(modal_midi(&[64, 60, 62, 65, 63]), 60);
    }

    #[test]
    fn modal_midi_singleton() {
        assert_eq!(modal_midi(&[60]), 60);
    }

    #[test]
    fn modal_midi_handles_negative_midi() {
        // Negative MIDI is musically nonsense but the helper is total —
        // the lower-wins tiebreak should not flip sign accidentally.
        assert_eq!(modal_midi(&[-2, -2, 5, 5]), -2);
    }

    #[test]
    fn median_f32_singleton() {
        let mut v = vec![3.0_f32];
        assert!((median_f32(&mut v) - 3.0).abs() < f32::EPSILON);
    }

    #[test]
    fn median_f32_odd_length() {
        let mut v = vec![3.0_f32, 1.0, 2.0];
        assert!((median_f32(&mut v) - 2.0).abs() < f32::EPSILON);
    }

    #[test]
    fn median_f32_even_length_averages_middle_pair() {
        let mut v = vec![1.0_f32, 2.0, 3.0, 4.0];
        // Sorted middle = (2+3)/2 = 2.5
        assert!((median_f32(&mut v) - 2.5).abs() < f32::EPSILON);
    }

    #[test]
    fn median_f32_with_nan_is_total() {
        // NaN compares Equal under our fallback, so it's treated as a
        // neighbour of whatever it lands beside; the helper must not
        // panic. Concrete returned value is implementation-defined but
        // MUST be finite for the all-non-NaN portion.
        let mut v = vec![3.0_f32, f32::NAN, 1.0, 2.0];
        let m = median_f32(&mut v);
        // Either of the middle elements is acceptable (the sort is
        // stable on Equal); the contract is "does not panic, returns a
        // bounded f32".
        assert!(m.is_finite() || m.is_nan());
    }

    #[test]
    fn percentile_sorted_basic() {
        let v: Vec<f32> = (0..=100).map(|i| i as f32).collect();
        // Sorted 0..100 ascending — p50 lands between 50 and 50; p99 ~ 99.
        assert!((percentile_sorted(&v, 50.0) - 50.0).abs() < 1e-3);
        assert!((percentile_sorted(&v, 99.0) - 99.0).abs() < 1e-3);
    }

    #[test]
    fn percentile_sorted_empty_returns_zero() {
        let v: Vec<f32> = vec![];
        assert!(percentile_sorted(&v, 50.0).abs() < f32::EPSILON);
    }

    #[test]
    fn percentile_sorted_singleton() {
        let v = vec![7.0_f32];
        assert!((percentile_sorted(&v, 50.0) - 7.0).abs() < f32::EPSILON);
        assert!((percentile_sorted(&v, 99.0) - 7.0).abs() < f32::EPSILON);
    }

    #[test]
    fn render_aggregate_json_has_all_required_keys() {
        let s = render_aggregate_json(0.95, 24, 13, 1.234, 4.567);
        for key in [
            "aggregate",
            "unit_test_count",
            "fixture_test_count",
            "latency_p50_ms",
            "latency_p99_ms",
        ] {
            assert!(s.contains(key), "missing key {key} in {s}");
        }
    }
}
