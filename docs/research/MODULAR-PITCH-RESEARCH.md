# Modular Pitch Detection — Research Addendum

*Focused addendum to `RESEARCH-REPORT.md` covering: neural monophonic detectors (CREPE / PESTO / FCN-F0 / SPICE / torchcrepe), automatic instrument-prior elimination (YAMNet / PANNs / F0-histogram bootstrap), per-stem pitch-detection routing, sung-voice specialisation (vibrato + range), and the Rust trait surface that lets all of these plug into the existing pipeline without disturbing Phase 1. Date of synthesis: 2026-06-02.*

---

## Executive Summary

- **Manual instrument selector should not be the default UX.** Classical YIN / MPM detectors *need* a τ-range prior to avoid octave halving on vibrato and breathy onsets; modern neural detectors (CREPE, PESTO) learn the fundamental directly. Build a modular `PitchEstimator` trait so backends are swappable, and switch the default to auto-detect instead of forcing the user to pick Guitar / Bass / Voice.
- **Default live-tuner backend for the singing primary use case: PESTO (ONNX) with YIN/MPM as a parallel cross-check.** PESTO is the only neural detector in the survey that combines a documented in-repo ONNX export, measured ~0.7 ms streaming inference per frame on a laptop CPU (Intel i7-1185G7), a transposition-equivariant SSL training objective (ISMIR 2023 Best Paper), ~30 K params (v1) / ~130 K (v2), a native 48 kHz streaming chunk of 960 samples, and active 2025-2026 maintenance. YIN runs in ~50 µs/block in parallel as an octave-disagreement alarm.
- **PESTO is viable and is the headline architectural unlock.** The "90% smaller CREPE that runs real-time on CPU" claim survives verification: ~0.7 ms/frame (PESTO) vs one-to-two orders of magnitude slower (CREPE-full); the ONNX export script ships in-repo (`realtime/export_onnx.py`). The one real friction is LGPL-3.0 on the Python inference reference — addressable by shipping only the exported ONNX file as runtime data and writing a clean-room Rust `ort` host. Counsel sign-off recommended before public release.
- **Per-stem pipeline (Phase 4): different detector per stem, dispatched by a stem-aware aggregator.** Vocals → pYIN or CREPE/PESTO with Viterbi; Bass → wide-window YIN (4096 @ 48 kHz); Drums → onset detection + drum-class classifier (NOT pitch); Other → Basic Pitch per sub-stem via `htdemucs_6s`, with whole-mix Basic Pitch as fallback.
- **Auto-priors when classical fallback is needed**: YAMNet (3.7 M params, ~17 MB, Apache-2.0) tagging at 1 Hz + power-weighted F0 histogram bootstrap over the last 2–4 s + running-median dynamic τ prior. Eliminates the manual instrument selector for the YIN/MPM fallback path on high-confidence inputs.
- **Decoder choice is non-negotiable**: ALWAYS run Viterbi (or HMM) decoding on the model's logit output. Raw argmax reintroduces YIN-class octave-halving — the very failure the neural model was supposed to eliminate. The Viterbi port is ~50–100 LOC of DP in Rust; do not skip it.
- **Phase 1 still ships YIN/MPM first.** The trait-based architecture means Phase 2 adds CREPE-tiny and PESTO without touching Phase 1 calling code. The manual selector is removed for voice; it remains as an advanced toggle for guitar / bass / piano.

---

## 1. Why the Manual Instrument Selector Felt Wrong

The original Phase 1 design (§3 of `RESEARCH-REPORT.md`) makes the user pick an instrument profile (Guitar / Bass / Voice-low / Voice-high / Generic / Piano) before the tuner runs. YIN and MPM are *time-domain autocorrelation-class* detectors that pick the smallest dip below an absolute threshold in the difference function `d(τ)`. Without a τ-search range, the algorithm is free to lock onto a sub-harmonic (octave-down) or super-harmonic (octave-up), and this is its dominant failure mode on vibrato, breathy tone, and noisy onsets (de Cheveigné & Kawahara 2002). The instrument profile is therefore a **hard τ-range clamp** — but it forces the user to know what they are doing before they hear themselves.

Modern neural detectors do not work this way. CREPE is a 360-bin classifier head over cents (20 cents/bin) that outputs a softmax posterior — it can express bimodal posterior on octave ambiguity, and Viterbi decoding then resolves the ambiguity over time. PESTO is a transposition-equivariant CNN trained with an SSL objective whose entire point is that an octave-shifted input produces a translated output, so the model has *learned* what an octave is. Neither model needs an instrument prior to be octave-correct; the prior is at most a useful post-hoc clamp to discard impossible candidates, not a precondition for correctness.

The implication: the singing primary use case can default to "auto" and just work. The instrument selector remains reachable as an advanced setting for the deterministic YIN fallback, but it does not belong on the front of the live tuner.

---

## 2. Neural Monophonic Detectors — Comparison Table

All numbers below are from the cited primary sources (repo README, paper abstract, repo metadata via the GitHub API). Latency figures marked "extrapolated" are interpolations between published benchmarks and are flagged as such.

| Model | Year | Params | Latency on CPU (~ms / frame, 1024 @ 16 kHz) | Octave-error vs YIN | License | ONNX availability | Runs in Rust via `ort`? | Maintenance health |
|---|---|---|---|---|---|---|---|---|
| YIN / MPM (baseline) | 2002 / 2005 | n/a | ~0.05 ms (50 µs / 2048 block in Rust) | poor without HMM; dominant failure = octave halving | trivial / MIT (sevagh, alesgenova) | n/a | yes (pure Rust) | dormant but stable |
| pYIN (HMM-smoothed YIN) | 2014 | n/a | ~0.5–2 ms / frame | strong (Viterbi prior with `max_transition_rate` 35.92 oct/s default) | ISC (librosa) / MIT (Sytronik `pyin` Rust port) | n/a | yes (pure Rust) | active enough |
| CREPE — full (TF) | 2018 | ~22 M | ~50–150 ms (CPU, full model is huge) | strong with `--viterbi`; "double/half frequency errors" without (torchcrepe README) | MIT | yqzhishen/onnxcrepe v1.1.0 (2022-10-18); 89.0 MB ONNX | yes via `ort` | upstream dormant; community port still alive |
| CREPE — tiny (TF) | 2018 | ~487 K | ~5–15 ms (extrapolated; ort on laptop CPU) | strong with Viterbi | MIT | yqzhishen/onnxcrepe v1.1.0 tiny.onnx 1.96 MB | yes via `ort` | community port maintained |
| **PESTO v1** | 2023 | ~30 K | **~0.7 ± 0.03 ms / frame (i7-1185G7, README benchmark)** | strong (transposition-equivariant SSL + optional Viterbi) | LGPL-3.0 (inference repo); paper CC BY 4.0 | **yes — `python -m realtime.export_onnx`** | yes via `ort` (stateful: requires cache tensor threading) | active (last push 2025-10-15) |
| PESTO v2 | 2025 | ~130 K | sub-10 ms streaming on VQT (TISMIR 2025; re-benchmark on target HW) | strong | LGPL-3.0 | yes (same export path) | yes via `ort` | active |
| FCN-F0 (FCN-1953 / -993 / -929) | 2019 | small | FCN-993 0.89 s vs CREPE 14.79 s on cited test (much faster on CPU) | published RPA on synthetic speech only | MIT | **none in repo** | not without conversion | dormant (last push 2022-12) |
| SPICE | 2020 | small (CNN over CQT) | similar order to CREPE | reportedly comparable (self-supervised CQT-shift) | Apache-2.0 (code) / CC BY 4.0 (content) | **no first-party ONNX**; on Kaggle as TFLite + TF1 SavedModel | only via tf2onnx with TF1 shim — no community port | model alive, repo not |
| torchcrepe | 2018 / port 2025 | same as CREPE | same as CREPE | same as CREPE; default decoder is Viterbi | MIT | wraps CREPE weights in PyTorch (no first-party ONNX) | indirectly (export to ONNX yourself, then `ort`) | active (last push 2025-05-16, 517 stars) |

### 2.1 Recommendation for the live tuner backend

**Ship PESTO (v1) as the default neural detector for the live singing path, with YIN/MPM running in parallel as a deterministic cross-check.** Rationale: (1) PESTO is the only model with a documented in-repo ONNX export *and* measured sub-millisecond CPU streaming inference *and* a native 48 kHz path; every other neural option requires a 16 kHz resample first. (2) The transposition-equivariant SSL training objective is precisely the property that yields octave-robust outputs without an instrument prior. (3) Sub-50 ms perceived latency budget is comfortably met. (4) The LGPL-3.0 license is the only real friction — ship the trained ONNX as a downloaded asset, do not vendor any LGPL Python source, write a clean-room Rust `ort` host, get counsel sign-off before public release.

If counsel rejects LGPL-3.0 even for ONNX-as-data shipping, the fallback is **CREPE-tiny via yqzhishen/onnxcrepe (MIT, tiny.onnx 1.96 MB)** with a Rust port of the librosa Viterbi decoder (~50 LOC).

Do **NOT** ship FCN-F0 (speech-only; output capped at 1000 Hz; no ONNX), SPICE (no first-party ONNX; TF Hub URL has already broken once), or DDSP (it is a synthesizer, not a detector).

---

## 3. Auto-Priors and Instrument Classification

Even with PESTO as the default, the YIN/MPM fallback path still needs to behave well — partly for cross-checking, partly for low-CPU devices, and partly because future contributors will plug in additional classical detectors via the trait abstraction (§6).

### 3.1 Three-stage auto-prior pipeline

| Stage | Mechanism | Cost | Output |
|---|---|---|---|
| 1. Coarse instrument tag | YAMNet (Apache-2.0, 3.7 M params, ~17 MB ONNX, 521 AudioSet classes) on a 0.96 s sliding buffer at 16 kHz, run every ~1 s | ~5–15 ms / inference / second | `{voice, guitar, bass, piano, drum, other}` macro-label + top-1 confidence + margin |
| 2. F0 histogram bootstrap | Wide first pass (fmin 27.5 Hz / fmax 4186 Hz) → power-weighted histogram over last 2–4 s → tightened second pass at median ± 1 octave (clamped by macro-label range) | ~0.5–2 ms / second | tightened (fmin, fmax) for stage 3 |
| 3. Dynamic τ prior | Running median of the last N=2 s of accepted f₀ frames + Gaussian penalty around median in YIN difference-function search (equivalent to pYIN's `max_transition_rate` 35.92 oct/s default) | negligible | per-frame f₀ with octave-jump rejection |

Confidence gate: when YAMNet top-1 confidence < 0.30 OR margin over runner-up < 0.10 OR the histogram is bimodal across more than an octave, fall back to the Generic 60–2000 Hz prior and surface a one-tap UI hint "Tap to set instrument".

### 3.2 Why YAMNet over PANNs

| Classifier | Params | Disk | mAP (balanced) | License | Verdict |
|---|---|---|---|---|---|
| YAMNet | 3.7 M | ~17 MB | 0.306 | Apache-2.0 | Default — exact taxonomy needed (Singing 24, Guitar 135, Electric guitar 136, Bass guitar 137, Acoustic guitar 138, Piano 148, Drum 159, Violin/fiddle 186, etc.) |
| PANNs CNN14 | tens of M | 327 MB | 0.431 | code MIT / weights CC BY 4.0 | Too large for a desktop tuner bundle |
| PANNs MobileNetV1 | ~2 M | 23.6 MB | 0.389 | code MIT / weights CC BY 4.0 | Phase-2 upgrade if YAMNet's confidence distribution is the limiting factor |
| PANNs Wavegram-Logmel-CNN | tens of M | 328.7 MB | 0.439 | code MIT / weights CC BY 4.0 | Server-only |

YAMNet collapses 521 classes from the original 527 AudioSet labels — it merges "Male singing" / "Female singing" into "Singing", which is fine for instrument-tag purposes but loses gendered vocal classes that PANNs preserves.

### 3.3 What NOT to do for vocal detection

Do **not** run Spleeter or Demucs on the live path to get a "vocal confidence" number. Demucs is ~1.5× slower than real-time on CPU per its own README; demucs.cpp trades speed for memory; Spleeter advertises only GPU performance. YAMNet's "Singing" / "Choir" / "Humming" class probabilities are the cheapest credible vocal-presence signal for live use. Silero VAD (~2 MB ONNX, MIT) is a *speech* VAD trained on speech-in-noise, not a sung-voice-in-music detector — the wrong tool for "is the input a vocal melody".

---

## 4. Per-Stem Pitch Detection Pipeline

The Phase 4 architecture (stem separation → per-stem analysis) needs a different detector per stem because stems have different acoustic statistics. Routing matters more than the underlying choice of any single model.

### 4.1 Stem-to-detector dispatch table

| Stem | Recommended detector | Rationale |
|---|---|---|
| **Vocals** | pYIN (offline, librosa via PyO3 in eval; Sytronik `pyin` Rust crate in production) **or** CREPE-tiny/PESTO with Viterbi | pYIN's HMM is the published gold standard for vocal F0 and runs trivially on CPU; CREPE/PESTO win on noisy or breathy vocals. Default to pYIN; escalate to neural when pYIN voiced_prob < 0.3 for >500 ms. |
| **Bass** | Wide-window YIN (4096 @ 48 kHz) **or** PENN/FCNF0++ (frequency range 31–1984 Hz, MIT) | Bass low E1 ≈ 41 Hz needs ≥ 4096 sample window for ≥ 2 periods; classical YIN is fine here because the bass timbre has strong fundamental and weak high harmonics, so octave-up errors are rare. |
| **Drums** | **NOT pitch** — onset detection + drum-class classifier | Drums have no meaningful F0. Use ADTLib (BSD-2-Clause, 3-class kick/snare/hihat) for the license-clean baseline; if 5-class is needed, retrain Magenta Onsets-and-Frames-drums (Apache-2.0) on E-GMD yourself — ADTOF is CC-BY-NC-SA 4.0 and is a hard blocker for commercial use. |
| **Other** | Basic Pitch per sub-stem (use `htdemucs_6s` to split "other" into guitar + piano + true-other) **or** whole-mix Basic Pitch as fallback | The "other" stem is universally weakest (5–8 dB SDR vs 10–12 for drums/bass/vocals) because it is a residual catch-all. Splitting it into 6-stem reduces this to two viable sub-stems (guitar, piano) plus a residual; Basic Pitch handles each sub-stem better than the full residual. MT3 (Apache-2.0, ICLR 2022) is the server-side / GPU pro-mode escape hatch when local Basic Pitch is insufficient — no ONNX export, JAX/Flax only. |
| **Whole mix (fallback)** | Basic Pitch | When stem separation fails or is skipped (low-CPU mode, user explicit choice), fall back to whole-mix Basic Pitch with a UI caveat that polyphonic accuracy is degraded. |

### 4.2 Octave-error budget per stem

For stems with strong harmonic content (vocals, "other" with guitars), Viterbi/HMM smoothing is the dominant octave-error mitigation — pYIN bakes it in via `max_transition_rate`, torchcrepe defaults to Viterbi, PESTO ships it as an optional post-process, PENN ships `torbi` for batched Viterbi. For bass, classical YIN with a wide window is fine because there is no strong second harmonic to mistake for the fundamental.

### 4.3 What to skip

- **htdemucs_6s** is the right separator for §4.1 (guitar + piano sub-stems for free), but slowest. Low-CPU tiers fall back to 4-stem htdemucs.
- **MT3** is server-only; no ONNX; README says training is unsupported.
- **Open-Unmix UMX-L** — already excluded from §5 of the main report (CC-BY-NC-SA 4.0 weights).

---

## 5. Voice / Singing — Specialized Recommendations

The singing primary use case has three sub-problems beyond plain pitch detection: (a) tracking sung pitch with vibrato, (b) detecting the singer's range, (c) presenting pitch contour in a UX the user can read and react to in real time.

### 5.1 pYIN vs CREPE vs PESTO for sung pitch

- **pYIN** is the strongest classical baseline. Beta-distribution prior over the YIN difference-function threshold + Boltzmann prior over period candidates + Viterbi smoothing — the published countermeasure to vibrato-induced octave errors. CPU-only, no model weights.
- **CREPE** with Viterbi is statistically stronger than pYIN on noisy material per the 2018 paper. Raw argmax has documented double/half-frequency errors and is *worse* than pYIN-with-Viterbi on noisy singing — Viterbi is non-optional.
- **PESTO** has the smallest model footprint of the three. The streaming wrapper handles vibrato natively because the network sees a full-rate VQT slice every 960 samples and the SSL objective is invariant to small pitch translations.

Live tuner ordering: PESTO (default) → pYIN (offline / cross-check) → YIN (ultra-low-CPU fallback). For offline recording analysis pYIN gives publication-grade contours with no model-weight licensing risk.

### 5.2 Vibrato detection (4–7 Hz FFT-of-residual)

Standard technique: over a stable note region (50–500 ms minimum), subtract a median-filtered or polynomial-fit baseline (the "intended" pitch) from the f₀ contour, FFT the residual, look for a peak in the 4–7 Hz band. Vibrato rate = peak frequency; extent = twice the peak amplitude in cents. Operatic vibrato is 5–7 Hz / ±50 cents; pop is 4–6 Hz / ±20–30 cents. Smooth to ~100 Hz frame rate, baseline-subtract over a sliding 200–500 ms window, FFT a 1–2 s window of residual. Display vibrato as a separate UI element — it is musical intent, not an error.

### 5.3 Vocal range detection algorithm

1. Run pYIN/PESTO over the session, keep voiced frames with confidence > 0.7.
2. Convert f₀ to MIDI cents; histogram with 1-semitone bins.
3. Trim 1% from each tail to discard vocal-fry / falsetto outliers.
4. Report trimmed min/max as *comfortable* range, untrimmed as *full* range.
5. For voice-type (Bass / Baritone / Tenor / Alto / Mezzo / Soprano), compare comfortable range to New Grove ranges: Bass E2–E4, Baritone G2–G4, Tenor C3–C5, Alto F3–F5, Mezzo A3–A5, Soprano C4–C6. Allow ±1 semitone tolerance; report ambiguous cases as "between Tenor and Baritone" rather than forcing a choice.

### 5.4 Pitch contour visualization patterns

- **Smule / SingStar** — horizontal target bars + live pitch dot + tolerance window with green-fill when in tune. Tolerance ±50 cents typical, ±20 cents in "hard mode".
- **Yousician** (Vocal mode) — same ribbon pattern + explicit "you sang this much of this note in tune" feedback after each phrase.
- **Melodyne** (offline) — blob + intra-note pitch curve. Complementary to the live tuner.

Smule's target-bar pattern is the universal mental model for sung-pitch feedback. Do not invent a new visualization.

---

## 6. Modular Architecture — Rust Trait Design

The pipeline should not care which estimator is running. Phase 1 ships YIN/MPM; Phase 2 adds CREPE-tiny and PESTO; Phase 4 routes per-stem. With a trait abstraction, each addition is a new file plus a registration line — no calling-code refactor.

### 6.1 The `PitchEstimator` trait

```rust
//! core/src/pitch/mod.rs

use std::sync::Arc;

/// One frame's worth of pitch information.
#[derive(Clone, Copy, Debug)]
pub struct F0Frame {
    pub f0_hz: f32,
    pub confidence: f32,           // 0.0 .. 1.0
    pub voiced: bool,              // gated on confidence + RMS
    pub timestamp_samples: u64,    // monotonic, sample-accurate
}

/// Static configuration carried by every estimator.
#[derive(Clone, Debug)]
pub struct EstimatorConfig {
    pub sample_rate_hz: u32,        // 48 000 typical
    pub window_size: usize,         // estimator-specific (1024 / 2048 / 4096)
    pub hop_size: usize,            // estimator-specific
    pub fmin_hz: f32,               // soft prior; estimators MAY ignore (e.g. PESTO)
    pub fmax_hz: f32,
    pub instrument_hint: Option<InstrumentHint>,
}

#[derive(Clone, Copy, Debug)]
pub enum InstrumentHint {
    Voice, Guitar, Bass, Piano, Violin, Generic,
}

/// Errors a backend can produce at construction or at inference.
#[derive(thiserror::Error, Debug)]
pub enum EstimatorError {
    #[error("model file not found: {0}")] ModelNotFound(std::path::PathBuf),
    #[error("ort runtime error: {0}")] Ort(String),
    #[error("input frame size {got} != expected {want}")] WindowMismatch { got: usize, want: usize },
    #[error("backend disabled at compile time: {0}")] FeatureDisabled(&'static str),
}

/// The core trait. Note: stateful (PESTO needs cache; pYIN needs HMM history).
pub trait PitchEstimator: Send {
    /// Backend-stable name, used for telemetry and license-register lookup.
    fn name(&self) -> &'static str;

    /// What the caller must feed in.
    fn config(&self) -> &EstimatorConfig;

    /// Process exactly one window. May return None when voiced=false
    /// and the backend has no useful confidence to report.
    fn process(&mut self, samples: &[f32]) -> Result<Option<F0Frame>, EstimatorError>;

    /// Reset internal state (cache tensors, HMM history, smoothers) on
    /// stream restart, instrument change, or seek.
    fn reset(&mut self);
}
```

Three properties matter: (1) **`&mut self` on `process`** — PESTO's ONNX is *stateful* via an external cache tensor (`StatelessPESTO` wrapper); pYIN carries HMM history; CREPE-tiny is stateless and ignores the mutability. (2) **`Option<F0Frame>` return** — voiced/unvoiced is a per-backend decision. (3) **`Send` bound, no `Sync`** — estimators are owned by the DSP worker thread; no `Mutex` overhead.

### 6.2 The estimator factory

```rust
//! core/src/pitch/factory.rs

pub enum Backend {
    YinMpm,
    PYin,
    OnnxCrepeTiny,
    OnnxPesto,
}

pub fn make_estimator(
    backend: Backend,
    cfg: EstimatorConfig,
    model_root: &std::path::Path,
) -> Result<Box<dyn PitchEstimator>, EstimatorError> {
    match backend {
        Backend::YinMpm        => Ok(Box::new(yin_mpm::YinMpmEstimator::new(cfg))),
        Backend::PYin          => Ok(Box::new(pyin::PYinEstimator::new(cfg))),
        #[cfg(feature = "neural")]
        Backend::OnnxCrepeTiny => Ok(Box::new(neural::CrepeTinyEstimator::new(cfg, model_root)?)),
        #[cfg(feature = "neural")]
        Backend::OnnxPesto     => Ok(Box::new(neural::PestoEstimator::new(cfg, model_root)?)),
        #[cfg(not(feature = "neural"))]
        _ => Err(EstimatorError::FeatureDisabled("neural")),
    }
}
```

The `feature = "neural"` cfg gate is deliberate: low-CPU / mobile builds drop the `ort` dependency entirely and ship YIN/MPM + pYIN only.

### 6.3 Pipeline composition

```rust
//! core/src/pipeline/live_tuner.rs

pub struct LiveTunerPipeline {
    estimator: Box<dyn PitchEstimator>,
    smoother: ContourSmoother,         // 200–500 ms running mean of cents
    sink: tauri::ipc::Channel<PitchUpdate>,
}

impl LiveTunerPipeline {
    pub fn run(mut self, mut audio_in: rtrb::Consumer<f32>) {
        let win = self.estimator.config().window_size;
        let hop = self.estimator.config().hop_size;
        let mut buf = vec![0.0_f32; win];
        loop {
            // ... drain `hop` samples from audio_in, slide buf left by hop ...
            if let Some(frame) = self.estimator.process(&buf).ok().flatten() {
                let smoothed = self.smoother.push(frame);
                let _ = self.sink.send(PitchUpdate::from(smoothed));
            }
        }
    }
}
```

The pipeline does not know which backend is running. Swapping YIN for PESTO is `make_estimator(Backend::OnnxPesto, ...)`. Phase 1 calling code never changes.

### 6.4 Per-stem song analysis

```rust
//! core/src/pipeline/song_analysis.rs

pub struct StemSpec {
    pub name: StemName,                            // Vocals, Bass, Drums, Other
    pub estimator: Box<dyn PitchEstimator>,        // None for Drums (uses OnsetDetector trait)
    pub onset_detector: Option<Box<dyn OnsetDetector>>,
}

pub struct SongAnalysisPipeline {
    pub separator: Box<dyn StemSeparator>,         // Demucs htdemucs / htdemucs_6s
    pub stems: Vec<StemSpec>,                      // one per stem the separator emits
    pub aggregator: NoteEventAggregator,           // merges into a single MIDI/MusicXML output
}

impl SongAnalysisPipeline {
    pub fn analyze(&mut self, mix: &[f32]) -> SongScore {
        let stem_audio = self.separator.separate(mix);   // e.g. {vocals, drums, bass, other}
        let mut events = Vec::new();
        for spec in &mut self.stems {
            let stem = stem_audio.get(spec.name);
            if let Some(estimator) = spec.estimator.as_mut() {
                events.extend(run_estimator_over_stem(estimator, stem));
            }
            if let Some(onset) = spec.onset_detector.as_mut() {
                events.extend(run_onset_over_stem(onset, stem));
            }
        }
        self.aggregator.merge(events)
    }
}
```

The dispatch table from §4.1 becomes a vector of `StemSpec` values. Adding 6-stem support is adding two more entries (guitar, piano), each with a Basic Pitch estimator. The pipeline shape does not change.

### 6.5 Phase additivity

| Phase | Adds | Trait code touched |
|---|---|---|
| 1 | YinMpmEstimator | new file `yin_mpm.rs` + factory arm |
| 2 | PYin, CrepeTiny, Pesto | three new files + factory arms under `feature = "neural"` |
| 3 | Basic Pitch as polyphonic — separate `PolyEstimator` trait | mono trait unchanged |
| 4 | StemSpec + SongAnalysisPipeline | new pipeline type; estimator trait unchanged |

---

## 7. Recommended Backend Defaults Per Mode

| Mode | Default backend | Window | Notes |
|---|---|---|---|
| Live tuner — voice (DEFAULT) | **PESTO ONNX** + YIN cross-check | PESTO: 960 @ 48 kHz native; YIN: 2048 @ 48 kHz | YIN runs in parallel on the same audio; UI shows PESTO; raise an octave-disagreement flag if they differ by > 1 octave for > 200 ms |
| Live tuner — guitar (explicit) | YinMpm with Guitar prior (80–1300 Hz) | 2048 @ 48 kHz | Deterministic, low-CPU, exposed via "advanced" toggle |
| Live tuner — bass (explicit) | YinMpm with Bass prior (30–500 Hz) | 4096 @ 48 kHz | Wider window required for E1 ≈ 41 Hz |
| Live tuner — instrument auto-detect | YAMNet → macro-label → tightened YinMpm prior + dynamic τ; escalate to PESTO if available | varies | Confidence gate < 0.30 → fall back to Generic prior |
| Offline recording analysis (sung) | pYIN | 4096 @ 48 kHz (or 2048 @ 22.05 kHz to match Basic Pitch) | HMM-smoothed; no real-time constraint; deterministic |
| Offline song — vocal stem | pYIN (default), CREPE-tiny+Viterbi (escalation) | 4096 @ 48 kHz / 1024 @ 16 kHz | pYIN voiced_prob < 0.3 for >500 ms triggers CREPE escalation |
| Offline song — bass stem | YinMpm with Bass prior, wide window | 4096 @ 48 kHz | Optionally PENN (frequency range 31–1984 Hz) for cross-domain accuracy |
| Offline song — drum stem | OnsetDetector + ADTLib-class drum classifier | n/a | Pitch reporting suppressed |
| Offline song — other stem | Basic Pitch per sub-stem (with `htdemucs_6s`) or per whole "other" stem (with 4-stem) | Basic Pitch native (43844-sample window @ 22.05 kHz) | UI labels the output as "residual instruments" |

All per-mode defaults flow through the same `PitchEstimator` trait. The mode picks the `Backend` enum and the `EstimatorConfig`; everything else is uniform.

---

## 8. Risks and Open Questions

### 8.1 Verifier-corrected positions

| Original payload claim | Verdict | Corrected position |
|---|---|---|
| PESTO weights are "data" not "covered work" under LGPL §6 | partially | Paper is CC BY 4.0; weights' license status is contested same as MUSDB-derived weights in §14 of the main report. Conservative: ship weights as runtime asset, no LGPL Python vendoring, get counsel sign-off. |
| PESTO ONNX is 0.7 ms/frame on laptop CPU | confirmed for v1 (i7-1185G7) | README benchmark is for v1. v2 (~130 K params) is newer; re-benchmark on target hardware. |
| CREPE outperforms pYIN unconditionally | partially | Verified on cited 2018 datasets; noise robustness contested in cross-talk / narrowband (Kato & Kinnunen 2019). |
| Basic Pitch is "best on one instrument at a time" | strong (Spotify README) | Per-stem Basic Pitch is the right architecture; whole-mix is the v1 floor. |
| ADTOF is the right drum transcriber | refuted for commercial | CC-BY-NC-SA 4.0 — hard blocker. Use ADTLib (BSD-2-Clause, 3-class); retrain Magenta O&F-drums on E-GMD for 5-class. |
| YAMNet mAP transfers to music classes | weak | 0.306 is averaged across 521 classes; per-class music numbers not broken out. Benchmark on your own corpus. |
| DDSP is a pitch detector | refuted | DDSP is a synthesizer. Its `pitch_detection.ipynb` demos others' models; canonical DDSP upstream detector is CREPE. |
| Demucs gives real-time CPU stem separation | refuted | ~1.5× *slower* than real-time on CPU per README; demucs.cpp trades speed for memory. Per-stem is an *offline* feature. |

### 8.2 ONNX-export gaps

| Model | Export path | Risk |
|---|---|---|
| CREPE | yqzhishen/onnxcrepe v1.1.0 (2022) | Community port; unmaintained; bug fixes require forking. tiny 1.96 MB → full 89.0 MB. |
| PESTO | first-party `python -m realtime.export_onnx` | Stateful via cache tensor — Rust caller MUST thread `cache_in` → `cache_out`. |
| FCN-F0 | none | Speech-only model, not worth the TF1 conversion bug surface. |
| SPICE | none first-party; Kaggle TFLite + TF1 SavedModel | TF1-compatibility shim required; no community port. |
| PENN | none documented | Pure PyTorch; `torch.onnx.export` should work but untested. |
| YAMNet | tf2onnx community ports | Not first-party — validate scores on AudioSet eval before shipping. |
| Basic Pitch | first-party ONNX in repo | Covered in §4 of original report. |
| Demucs | sevagh/demucs.onnx (POC) | STFT/iSTFT pulled out of graph; demucs.cpp (MIT, GGML) is the mature native alternative. |

### 8.3 CPU-cost reality vs marketing

PESTO's 0.7 ms/frame is on i7-1185G7. On Apple M1 similar or faster; on low-end Intel N100 / 2019 mobile CPU 2–4× slower; on Android arm64-v8a expect 4–8× slower (~3–6 ms/frame), still well within sub-50 ms. CREPE-full's 89 MB ONNX is a non-starter for a mobile bundle; tiny (1.96 MB) is fine. YAMNet at 17 MB is desktop-only by mobile bundle standards; on mobile, fall back to a rule-based RMS-by-band instrument tagger or require explicit selection.

### 8.4 Open questions

- **PESTO LGPL counsel review.** Required before public release. If rejected, fall back to CREPE-tiny + onnxcrepe weighted_viterbi.
- **Per-class YAMNet precision on music inputs.** Benchmark on your own corpus before trusting the auto-prior in production.
- **PESTO v2 latency.** README RTF is for v1; re-benchmark v2 on target hardware.
- **Viterbi decoder Rust port correctness.** ~50–100 LOC of DP; needs golden tests against librosa output.
- **`htdemucs_6s` ONNX export.** Single-author POC at sevagh/demucs.onnx; demucs.cpp is the more mature alternative for native Rust deployment.

---

## 9. Updated Phase Roadmap Implications

Deltas to §15 of `RESEARCH-REPORT.md`.

### Phase 1 — Live Monophonic Tuner (changes)

- **Default backend changes from "user-selected YIN with instrument prior" to "auto-instrument YIN/MPM with Generic prior"**. Selector moves to "Advanced settings".
- The `PitchEstimator` trait (§6) is introduced in this phase even though only `YinMpmEstimator` is implemented — ensures Phase 2 plug-in is mechanical.
- Add acceptance criterion: "octave-correctness ≥ 95% on Philharmonia voice fixtures with no manual instrument selection".

### Phase 2 — Record + Playback + Offline Note Display (additions)

- Add `PYinEstimator` (Sytronik `pyin` 1.2.0) for offline analysis.
- Add **vocal range detection** (§5.3) and **vibrato detection** (§5.2) as Phase 2 deliverables.
- Optionally add `OnnxCrepeTinyEstimator` and `OnnxPestoEstimator` behind a `feature = "neural"` flag — earliest neural backend appearance.

### Phase 3 — Polyphonic Transcription via Basic Pitch (unchanged)

Basic Pitch ONNX via `ort` 2.0 remains the v1 polyphonic feature. Polyphonic uses a separate `PolyEstimator` trait that is Phase 3-local.

### Phase 4 — Stem Separation + Per-Track Notes (significant changes)

- The dispatch table from §4.1 is the architectural deliverable: `SongAnalysisPipeline` (§6.4) with one `StemSpec` per emitted stem.
- Vocal: pYIN (default) + CREPE-tiny escalation. Bass: wide-window YIN. Drum: ADTLib onset detector — pitch *suppressed*. Other: Basic Pitch per sub-stem or whole-other.
- Add criteria: "drums never report pitch" and "bass octave-correctness ≥ 95% on synthesized E1–E3 stems".

### Phase 5 — Ear-Training Games (additions)

- Vocal-mode ear-training uses the Smule-style target-bar pattern from §5.4 — Phase 1 keeps the centred-needle tuner; Phase 5 adds the karaoke ribbon.
- Vocal range from Phase 2 gates exercise selection — sopranos do not get exercises that descend to E2.

### Phase 6 — Mobile (caveats)

- YAMNet (17 MB) is too heavy for mobile bundles; on mobile default to manual selector + Generic prior, OR a rule-based RMS-by-band tagger.
- PESTO and CREPE-tiny are mobile-friendly. CREPE-full / medium / large are desktop-only.

---

## 10. References

**Neural monophonic detectors** — Kim et al., "CREPE" (ICASSP 2018, arXiv:1802.06182); `github.com/marl/crepe` (MIT, TF, dormant), `github.com/maxrmorrison/torchcrepe` (MIT, active), `github.com/yqzhishen/onnxcrepe` (MIT, ONNX v1.1.0 2022-10-18). Riou, Lattner, Hadjeres, Peeters, "PESTO" (ISMIR 2023 Best Paper, arXiv:2309.02265); PESTO v2 arXiv:2508.01488 (TISMIR 2025); `github.com/SonyCSLParis/pesto` (LGPL-3.0, active 2025-10-15). Gfeller et al., "SPICE" (TASLP 2020, arXiv:1910.11664); Kaggle (formerly TF Hub). Ardaillon & Roebel, "FCN-F0" (Interspeech 2019); `github.com/ardaillon/FCN-f0` (MIT, dormant 2022-12). Mauch & Dixon, "pYIN" (ICASSP 2014); `librosa.pyin` (ISC). de Cheveigné & Kawahara, "YIN" (JASA 2002). McLeod & Wyvill, "A Smarter Way to Find Pitch" (ICMC 2005). Engel et al., "DDSP" (ICLR 2020) — *synthesis library, not a detector.*

**Auto-instrument classification / VAD** — YAMNet (`github.com/tensorflow/models/tree/master/research/audioset/yamnet`, Apache-2.0; class map `yamnet_class_map.csv`). Kong et al., PANNs (arXiv:1912.10211); `github.com/qiuqiangkong/audioset_tagging_cnn` (MIT code / CC BY 4.0 weights on Zenodo 3987831). Silero VAD (`github.com/snakers4/silero-vad`, MIT). Essentia models (`essentia.upf.edu/models.html`; MTG-trained models CC-BY-NC-SA 4.0 — non-commercial).

**Cross-domain pitch** — Morrison, Bryan, Pardo, "Cross-Domain Neural Pitch and Periodicity Estimation" (arXiv:2301.12258); PENN `github.com/interactiveaudiolab/penn` (MIT).

**Source separation (per-stem)** — Défossez, Hybrid Demucs (arXiv:2111.03600); Rouard et al., HT Demucs (arXiv:2211.08553); `github.com/facebookresearch/demucs` (archived 2026-01-01) and `github.com/adefossez/demucs` (maintenance). demucs.onnx fork (`github.com/sevagh/demucs.onnx`, MIT POC); demucs.cpp (`github.com/sevagh/demucs.cpp`, MIT, GGML). Lu et al., BS-RoFormer (arXiv:2309.02612); ZFTurbo `Music-Source-Separation-Training`.

**Drum transcription** — ADTLib `github.com/CarlSouthall/ADTLib` (BSD-2-Clause, 3-class). ADTOF `github.com/MZehren/ADTOF` (CC-BY-NC-SA 4.0 — non-commercial blocker). Magenta Onsets-and-Frames drums (Apache-2.0; train on E-GMD for 5-class).

**Polyphonic transcription** — Bittner et al., Basic Pitch (ICASSP 2022, arXiv:2203.09893; `github.com/spotify/basic-pitch`, Apache-2.0). Gardner et al., MT3 (ICLR 2022, arXiv:2111.03017; `github.com/magenta/mt3`, Apache-2.0, JAX/Flax only).

**Rust ML inference and audio** — ort (`github.com/pykeio/ort`, MIT/Apache-2.0, v2.0.0-rc.12); tract (`github.com/sonos/tract`, MIT/Apache-2.0); `librosa.sequence.viterbi` (ISC); `github.com/sevagh/pitch-detection` (MIT, MPM/YIN reference for clean-room Rust port).

**License notes** — LGPL-3.0 §6 dynamic-linking interpretation: not verified by counsel here; conservative posture for PESTO is to ship weights as runtime data, not vendor any LGPL Python, and get counsel sign-off before public release. aubio GPL-3.0 (closed-source incompatible); Essentia MTG models CC-BY-NC-SA 4.0 (non-commercial); ADTOF CC-BY-NC-SA 4.0 (non-commercial); ADTLib BSD-2-Clause; YAMNet Apache-2.0; PANNs MIT code / CC BY 4.0 weights; PESTO paper CC BY 4.0 / inference repo LGPL-3.0 / weights contested; CREPE & torchcrepe & onnxcrepe MIT; Basic Pitch Apache-2.0; Silero VAD MIT.
