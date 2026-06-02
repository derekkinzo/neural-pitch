# Neural-Pitch Research Report

*Synthesis of fan-out research, adversarial verification, and gap analysis for a Tauri + Rust pitch-detection / transcription / ear-training application. Date of synthesis: 2026-06-02.*

---

## Executive Summary

- **Stack**: Tauri 2 (desktop, mobile-aspirational) + a pure-Rust core crate. Frontend: SvelteKit 5 in static-SPA mode + Vite (default), with React 19 + Vite as the team-preference fallback. ML inference: `ort` 2.0 (release-candidate) wrapping ONNX Runtime, with `tract` (Sonos, pure-Rust) as a fallback for mobile binary-size or static-link constraints.
- **Live monophonic pitch (Phase 1)**: McLeod (MPM) or YIN at 48 kHz, 2048-sample analysis window, 512-sample hop, instrument-range priors, smoothed cents-deviation display. The `pitch-detection` crate (alesgenova, MIT/Apache, but stagnant since 2022) plus `pyin` (Sytronik, MIT, 2024) cover the algorithm needs.
- **Polyphonic transcription (Phase 3)**: Spotify Basic Pitch (Apache-2.0, ICASSP 2022) shipped as ONNX (≈ 225 KB on disk, ~17K parameters, instrument-agnostic). Acceptable-CPU path; not real-time.
- **Stem separation (Phase 4)**: Hybrid-Transformer Demucs v4 (`htdemucs` / `htdemucs_ft`, MIT, 9.0 dB SDR on MUSDB18-HQ) as a license-clean baseline; BS-RoFormer / Mel-Band RoFormer (lucidrains code MIT + community ZFTurbo weights MIT) when GPU is available and quality matters (≈ 9.65–10+ dB).
- **Top risks**: (1) **model-weight licensing** (MUSDB18 NC training data, UMX-L CC BY-NC-SA, Demucs weights without an explicit weight license, MAESTRO CC BY-NC-SA on MAESTRO-trained models) — commercial distribution is legally unsettled. (2) Tauri WebView accessibility is platform-fractured and silently regresses (open issue #12901: NVDA stops reading frameless windows after 2.3.0). (3) `cpal` is the only realistic cross-platform Rust audio I/O crate but its sole maintainer is publicly seeking handoff (issue #981). (4) `facebookresearch/demucs` was archived 2025-01-01; `adefossez/demucs` is in maintenance-only mode; the upstream PyPI release `demucs 4.0.1` is from September 2023. (5) Several digest claims on RPA/RMSE numbers and library-comparison superlatives did not survive verification — see §14.
- **Top recommendations**: (a) start with a tightly-scoped Phase 0 TDD harness on synthesized signals; (b) ship a live YIN/MPM tuner before any neural model; (c) build the analysis layer as a layered-pane / plugin-host so future modules (Basic Pitch, Demucs) plug in without UI rewrites; (d) maintain a versioned per-model License Register and have outside counsel sign off before any commercial release.

---

## 1. Project Goal & Scope

**neural-pitch** is a desktop-first, mobile-aspirational app that listens to or imports audio and shows what notes are being played or sung. Phased plan: (1) live monophonic tuner; (2) recording + playback with note display; (3) ear-training UI; (4) polyphonic transcription of imported songs; (5) stem separation with per-track notes; (6) iOS + Android ports of the same Rust core.

Methodology: research → design → TDD implementation. The Rust core is built standalone (no Tauri imports) so it can later be linked into mobile shells or a CLI. UI runs in Tauri's WebView; DSP runs in Rust threads behind `ipc::Channel<T>`. Use-case priorities: ear training / sight-singing first; stem separation + per-track notes second.

---

## 2. Recommended Stack

| Layer | Choice | License | Notes |
|---|---|---|---|
| Shell | Tauri 2.x (cli 2.11.x, May 2026) | Apache-2.0 / MIT | Desktop-stable since 2024-10-02; mobile DX still maturing per maintainers |
| Frontend framework | SvelteKit 5 + adapter-static + Vite (default); React 19 + Vite (fallback) | MIT | SvelteKit must run as static SPA in Tauri; SolidJS is also viable |
| Audio I/O | cpal 0.17.x (`realtime` feature where available) | Apache-2.0 | Cross-platform mic + speaker; sole maintainer seeking handoff (issue #981) |
| Decode | Symphonia 0.6.x | MPL-2.0 | WAV / FLAC / MP3 / OGG / AAC-LC / ALAC / M4A / AIFF / CAF, demuxer + decoder |
| FFT | rustfft 6.4.x + realfft 3.5.x | Apache-2.0 / MIT | Pure Rust, AVX/SSE/NEON SIMD |
| Resampling | rubato 3.0.x | MIT | Pure Rust; sinc / FFT / polynomial |
| SPSC ring | rtrb 0.3.4 (preferred) or ringbuf 0.4.0 | MIT / Apache-2.0 | Wait-free vs lock-free SPSC |
| Mel spectrogram | mel_spec 0.4.x | MIT | librosa / whisper.cpp parity, streaming |
| Tensors | ndarray 0.17.x | MIT / Apache-2.0 | Standard input type for ort/tract |
| ML inference | ort 2.0.0-rc.x (default) / tract 0.23.x (fallback) | MIT / Apache-2.0 | ort wraps ONNX Runtime; tract is pure-Rust |
| Pitch (mono, classical) | alesgenova `pitch-detection` 0.3.0 (MPM/YIN); Sytronik `pyin` 1.2.0 | MIT / Apache-2.0 | First crate stagnant since 2022 — vendor-and-patch posture |
| Polyphonic transcription | Basic Pitch ONNX (Spotify) | Apache-2.0 (code + repo) | ~225 KB on disk; per-note pitch bends |
| Stem separation | Demucs v4 (htdemucs) for license clarity; BS-RoFormer/Mel-Band RoFormer for quality | MIT (code) / unclear (weights) | See §14 — weight license is the central risk |
| Notation | OpenSheetMusicDisplay (OSMD) 1.9.x | BSD-3-Clause | MusicXML/MXL native; no built-in note-name localization |
| Waveform + spectrogram | wavesurfer.js 7.12.x + Spectrogram plugin | BSD-3-Clause | Web Worker FFT, mel/bark/erb scales |
| MIDI export | midly 0.5.3 (default) or midi-msg 0.9.x (MIT-only legal) | Unlicense / MIT | Pure Rust, SMF formats 0/1/2, 14-bit pitch bends |
| MusicXML export | hedgetechllc/musicxml 1.1.x | MIT | Pure Rust, .mxl support |
| Persistence | rusqlite 0.40 or sqlx 0.8 (sqlite feature) | MIT | Per-file analysis cache |
| i18n (JS shell) | FormatJS / react-intl (React) or svelte-i18n + intl-messageformat (Svelte) | BSD-3 / MIT | ICU MessageFormat |
| i18n (Rust core) | fluent-rs 0.17 + icu4x 2.x | Apache-2.0 / Unicode | Number formatting per locale |
| a11y testing | axe-core 4.12.x | MPL-2.0 | Runs inside WebView |

```toml
# Cargo.toml — illustrative core dependencies
[dependencies]
cpal = { version = "0.17", features = ["realtime"] }
symphonia = { version = "0.6", features = ["all-codecs", "all-formats"] }
rustfft = "6.4"
realfft = "3.5"
rubato  = "3.0"
rtrb    = "0.3"
mel_spec = "0.4"
ndarray = "0.17"
ort     = { version = "2.0.0-rc.12", features = ["load-dynamic"] }
midly   = "0.5"
rusqlite = { version = "0.40", features = ["bundled"] }
serde   = { version = "1", features = ["derive"] }
thiserror = "1"
```

---

## 3. Pitch Detection: Monophonic

For monophonic pitch detection in 2026 the field has cleanly split into two regimes — **time-domain (live)** and **deep-learning (offline)** — with classical spectral methods (cepstrum, HPS, FFT peak-picking) reduced to pedagogical baselines.

### 3.1 Algorithm comparison

| Algorithm | Domain | Window need | Cost (1 frame, 2 k window) | Octave-error robustness | License posture | Best for |
|---|---|---|---|---|---|---|
| Plain ACF | time | ≥ 2 periods of f_min | O(W²) (or O(W log W) with FFT) | poor (period-doubling) | trivial | pedagogical only |
| Cepstrum | freq | similar | one or a few FFTs | moderate | trivial | speech baseline |
| HPS | freq | one FFT | small | poor (weak / missing F0) | trivial | rough sketches |
| FFT peak + parabolic | freq | one FFT | very small | poor on harmonic timbres | trivial | pure-tone calibration |
| **YIN** (de Cheveigné & Kawahara, JASA 2002) | time | ≥ 2 periods of f_min | O(W²) naive; yinfft / yinfast O(W log W) | order-of-magnitude better than ACF | MIT (`yin`) / GPL-3.0 (aubio) | live tuning |
| **MPM / NSDF** (McLeod & Wyvill 2005) | time | ≥ 2 periods | O(W²) or FFT-accelerated | very good | MIT / Apache-2.0 (`pitch-detection`), Boost (cycfi/q has its own non-MPM detector) | live tuning, instruments |
| **pYIN** (Mauch & Dixon, ICASSP 2014) | time + HMM | ≥ 2 periods + a few smoothing frames | YIN cost + Viterbi | best classical baseline | ISC (librosa); MIT (Sytronik `pyin` for Rust) | offline melody |
| **CREPE** (Kim et al., ICASSP 2018) | deep CNN | 1024 samples @ 16 kHz | millions of MACs | strongest reported in noise | MIT (marl/crepe and torchcrepe) | offline accuracy |
| **SPICE** (Gfeller et al., TASLP 2020) | self-supervised CNN | CQT slices | similar to CREPE | comparable | TF Hub model — terms per page | self-supervised baseline |

### 3.2 Recommendation

- **Live tuner (Phase 1)**: MPM or YIN at **48 kHz, 2048-sample window, 512-sample hop**. Use parabolic interpolation around the chosen lag, gate on clarity (1 − d′(τ) for YIN, NSDF peak height for MPM), and clamp the τ search range using an instrument-profile prior (Guitar 80–1300 Hz, Bass 35–500 Hz, Voice-low 70–500 Hz, Voice-high 150–1500 Hz, Generic 60–2000 Hz). For `pitch-detection` (alesgenova), expect `POWER_THRESHOLD` and `CLARITY_THRESHOLD` to need empirical tuning per microphone.
- **Offline transcription (Phases 2/3)**: pYIN as a strong CPU classical baseline (`pyin` crate or PyO3 bridge to librosa), CREPE-full or torchcrepe-full when GPU acceleration is available and noise-robustness matters.

### 3.3 Octave-error mitigation

Documented and verified:

1. Use YIN's first-dip-below-threshold rule (typical absolute threshold 0.10–0.15) — do **not** pick the global minimum.
2. For MPM, follow McLeod & Wyvill's "key maximum" / clarity-threshold logic exactly; small re-implementations get this wrong.
3. Apply instrument-range priors (clamp τ search) — the single highest-leverage mitigation in practice.
4. Add HMM smoothing (pYIN) when offline latency budget allows.
5. CREPE smoothed with Viterbi decoding gives the strongest reported octave-error reduction in noisy audio (per Kim et al. 2018; note that independent work — Kato & Kinnunen 2019 — found CREPE's noise robustness contested in cross-talk/narrowband conditions).

### 3.4 Window-length / latency arithmetic

| Sample rate | Window | Window time | Hop | Frames/s | Min F0 (≥ 2 periods) |
|---|---|---|---|---|---|
| 44.1 kHz | 1024 | 23.2 ms | 256 | 172 | ~86 Hz |
| 44.1 kHz | 2048 | 46.4 ms | 512 | 86 | ~43 Hz |
| 48 kHz | 1024 | 21.3 ms | 256 | 187 | ~94 Hz |
| 48 kHz | 2048 | 42.7 ms | 512 | 94 | ~47 Hz |
| 48 kHz | 4096 | 85.3 ms | 1024 | 47 | ~24 Hz |

**Critical correction (verifier finding)**: a 1024-sample window at 44.1/48 kHz does **not** cover bass guitar low E (E1 ≈ 41 Hz). A standard 6-string guitar's low E is E2 ≈ 82 Hz, which a 2048-sample window handles. For bass instruments, use 4096 samples at 48 kHz and accept ~85 ms window latency.

---

## 4. Pitch Detection: Polyphonic

For shipping a polyphonic AMT feature in a Rust app, **Spotify Basic Pitch (ICASSP 2022) is the clear first feature**: officially distributed as Apache-2.0 ONNX (`basic_pitch/saved_models/icassp_2022/nmp.onnx`, 225 KB on disk), instrument-agnostic, actively maintained (5.1 k stars, last push 2025-11), and emits both a frame-level F0 grid and note events with per-note pitch bends.

### 4.1 Comparison

| Model | Year | Params | License | Distribution | MAESTRO Note F1 (piano) | Multi-instr | Real-time | Mobile-friendly |
|---|---|---|---|---|---|---|---|---|
| **Basic Pitch** | 2022 | ~17 K | Apache-2.0 | ONNX / TFLite / CoreML / TF SM | 70.9 (no-offset) | yes | no | yes (~225 KB) |
| **Onsets-and-Frames** (Hawthorne 2018/2019) | 2018/2019 | ~26 M | Apache-2.0 (code) | TF1 ckpt only (Magenta archived) | 95.32 (Onset F1) | piano only | no | no |
| **Kong et al. 2020** (high-resolution piano + pedals) | 2020 | tens of M | MIT | PyTorch | 96.72 (Onset F1) | piano only | no | no |
| **MT3** | 2022 | ~93.7 M (Table 4) | Apache-2.0 (code) | JAX / T5X only — no ONNX | 0.9455 onset F1 | yes (per-note instrument) | no | no |
| **Hawthorne 2021 transformer** | 2021 | T5-small | Apache-2.0 (code) | JAX / T5X only | 95.95 / 82.18 with offset+vel | piano only | no | no |
| **Omnizart** | 2021 | varies | MIT | TF, no ONNX | n/a in README | yes | no | no (ARM-mac broken) |
| **ReconVAT** | 2021 | small | unspecified | PyTorch | n/a | strings/piano/woodwinds | no | no |
| **Deep Salience** | 2017 | small | MIT | Keras/TF1 (abandoned) | n/a | salience grid only | no | no |
| **Classical NMF** | — | — | none | DIY | low | yes | no | yes |

**Verifier corrections** to digest claims:

- "Onsets-and-Frames remains the strongest published piano-only model" is false. Kong et al. 2020 reports **Onset F1 96.72 on MAESTRO V1.0.0** (vs. O&F 95.32). MT3 and Hawthorne 2021 transformer also match or exceed O&F on different metrics.
- The Magenta `magenta-js` repo and `onsets_frames_transcription` are inactive/archived but `jongwook/onsets-and-frames` is **not formally archived** (last commit July 2021).
- Basic Pitch's "~17K params" and "3 bins/semitone contour" are widely repeated but should be cited from the paper rather than secondary sources.

### 4.2 Recommendation

Ship Basic Pitch via `ort` (or `tract`) as the v1 polyphonic feature. Do **not** promise piano-quality transcription — Basic Pitch's MAESTRO Note-no-offset F1 of 70.9 is materially below piano-specialized models. If a piano "pro mode" is later required, evaluate **Kong et al. 2020** (`kongqiuqiang/high-resolution-piano-transcription`, MIT) as the tier-2 upgrade, not Onsets-and-Frames.

Defer MT3 to a server-side / GPU "pro mode" — it has no ONNX export, no PyPI release, the README says training is unsupported, and inference is officially Colab-only.

### 4.3 Open issues to track on Basic Pitch upstream

- Issue #190 (frame-level temporal drift: `hop_size ≠ kept_frames × FFT_HOP`) — active.
- Issue #164 (bundled `.tflite` reported 0 bytes) — packaging hygiene.
- Note: Basic Pitch is not real-time. Its README explicitly notes "the algorithms used by basic-pitch do not allow for real-time AMT," and recommends streaming windows from disk for long files.

---

## 5. Source / Stem Separation

### 5.1 SDR comparison (4-stem MUSDB18-HQ unless noted)

| Model | Year | Avg SDR (MUSDB-HQ) | Vocals | Drums | Bass | Other | License (code) | License (weights) | Notes |
|---|---|---|---|---|---|---|---|---|---|
| HTDemucs (`htdemucs`) | 2022 | 9.00 dB | 8.24 | 10.88 | 11.76 | 5.74 | MIT | unspecified (MUSDB-trained) | Default Demucs v4 |
| HTDemucs_ft | 2022 | 9.16+ dB | 8.38 | 11.13 | 11.96 | 5.85 | MIT | unspecified | 4× slower, per-stem fine-tuned |
| Hybrid Demucs v3 (`hdemucs_mmi`) | 2021 | 8.88 dB | 8.22 | 10.70 | 11.17 | 5.42 | MIT | unspecified | Lower VRAM than v4 |
| BS-RoFormer (community) | 2023 | 9.65 dB | 11.08 | 11.61 | 8.48 | 7.44 | MIT (lucidrains) | varies | SOTA family today |
| Mel-Band RoFormer (vocal, KimberleyJensen) | 2023+ | — | 10.98 (Multisong) | — | — | — | MIT (code) | varies | Vocal SOTA |
| SCNet XL IHF | 2024 | 10.08 dB | 11.42 | 11.81 | 9.23 | 7.88 | MIT | varies | Newer architecture |
| MDX23C | 2023 | 7.15 dB | 9.23 | 7.93 | 5.77 | 5.68 | MIT | open | TFC-TDF-U-Net successor |
| Open-Unmix UMX | 2019 | ~5.5 dB | 6.32 | 5.73 | 5.23 | 4.02 | MIT | MIT | LSTM, one model per stem |
| Open-Unmix UMX-HQ | 2019 | ~5.9 dB | 6.25 | 6.04 | 5.07 | 4.28 | MIT | MIT | HQ variant |
| **Open-Unmix UMX-L** | 2019 | ~6.3 dB | 7.21 | 7.15 | 6.02 | 4.89 | MIT | **CC BY-NC-SA 4.0** | **Non-commercial only** |
| Spleeter | 2019 | ~5.9 dB | 6.5 | 6.7 | 5.4 | 4.0 | MIT (code) | unspecified | TF, archived |

### 5.2 Recommendation

- **License-clean baseline**: Demucs v4 (`htdemucs` / `htdemucs_ft`) from the `adefossez/demucs` fork. CPU usable at ~1.5× real-time on a typical laptop (re-benchmark on target hardware — the figure is from the 2022 README and is not a verified primary source). GPU needs ~3 GB VRAM for the fast path.
- **Quality-first when GPU available**: BS-RoFormer / Mel-Band RoFormer using `lucidrains/BS-RoFormer` code + community weights from `ZFTurbo/Music-Source-Separation-Training`. +0.5 dB on multi-stem average and >2 dB on dedicated single-stem vocal models. *Audit each weight file's license individually — the weights and code are in different repos.*
- **Avoid UMX-L** in any commercial product — weights are CC BY-NC-SA 4.0.
- **Avoid Spleeter** for new integrations — last release Sep 2021, 243 open issues, Apple Silicon issues.

### 5.3 CPU vs GPU strategy

| Path | Throughput on typical laptop | When to use |
|---|---|---|
| Demucs v4 CPU | ~1.5× real-time (paper) | Offline batch processing, no GPU |
| Demucs v4 GPU (3 GB+) | ~10–20× real-time | Default GPU desktop |
| BS-RoFormer CPU | impractical (whole albums) | n/a |
| BS-RoFormer GPU | quality-critical pipelines | Quality mode |

The "**other**" stem is universally weakest (5–8 dB SDR vs 10–12 for drums/bass/vocals) because it is a residual catch-all category, not a coherent source. Communicate this in the UI ("other instruments") rather than implying clean separation.

### 5.4 Licensing reality

Per §14, **none of these weights have unambiguously commercial-redistributable licenses**: MUSDB18-HQ training corpus is "for educational purposes only…not…for commercial purpose without express permission" (bundles MedleyDB CC BY-NC-SA tracks); whether trained weights inherit the NC restriction is **unsettled law** (Creative Commons explicitly states no consensus). For a paid commercial tier, expect to either license a commercial-cleared model (AudioShake, Audionamix) or retrain on commercially-licensed stems.

---

## 6. Rust DSP & Audio I/O

| Crate | Purpose | Latest version | License | Mobile-friendly | Maintenance signal |
|---|---|---|---|---|---|
| symphonia | Decode WAV/FLAC/MP3/OGG/AAC-LC/ALAC/M4A | 0.6.0 (May 2026) | MPL-2.0 | yes (pure Rust) | Active; maintainer asking for help (#203) |
| rustfft | Complex FFT | 6.4.1 (Sep 2025) | MIT / Apache-2.0 | yes (AVX/SSE/NEON SIMD) | Active |
| realfft | Real-input FFT | 3.5.0 (Jun 2025) | MIT | yes | Active (small project) |
| rubato | Sample-rate conversion | 3.0.0 (May 2026) | MIT | yes | Active |
| biquad | IIR biquad filters | 0.6.0 (Mar 2026) | MIT / Apache-2.0 | yes (no_std) | Active |
| fundsp | Audio synthesis graph | 0.23.0 (Jan 2026) | MIT / Apache-2.0 | yes | Active |
| dasp | Sample/frame primitives | 0.11.0 (May 2020) | MIT | yes | **Stagnant** (no release since 2020 despite recent commits) |
| hound | WAV read/write | 3.5.1 (Sep 2023) | Apache-2.0 | yes | Stable; scope-complete |
| sonogram | Spectrograph image | 0.7.3 (Aug 2025) | GPL-3.0+ | yes | Niche; **GPL** |
| mel_filter | Mel filterbank | 0.1.1 (Jan 2021) | MIT | yes | **Abandoned** |
| mel_spec | Streaming mel spectrogram | 0.4.0 (Jun 2026) | MIT | yes | Active |
| ndarray | n-D arrays | 0.17.x (Jan 2026) | MIT / Apache-2.0 | yes | Active |
| nalgebra | Linear algebra | 0.35.0 (May 2026) | Apache-2.0 | yes | Active |
| pitch-detection (alesgenova) | YIN, MPM, ACF | 0.3.0 (Jun 2022) | MIT / Apache-2.0 | yes | **Stagnant** since 2022; vendor-and-patch |
| pyin (Sytronik) | pYIN | 1.2.0 (Jul 2024) | MIT | yes | Small audience but functional |
| aubio-rs | C aubio wrapper | 0.2.0 (Apr 2021) | **GPL-3.0** | C toolchain required | **Stagnant**; license blocker |
| essentia (lagmoellertim) | Essentia C++ wrapper | 0.1.5 (Jul 2025) | **AGPL-3.0** | painful | License blocker |
| cpal | Audio capture + playback | 0.17.3 (Feb 2026) | Apache-2.0 | yes (iOS/Android backends) | Maintainer seeking handoff (#981) |
| rtrb | Wait-free SPSC ring | 0.3.4 (Apr 2026) | MIT / Apache-2.0 | yes | Active |
| ringbuf | Lock-free SPSC ring | 0.4.0 (Apr 2024) | MIT / Apache-2.0 | yes | Slowing |
| rust-jack | JACK bindings | 0.13.x (Sep 2024) | MIT | Linux/macOS/Windows + JACK server | Active |
| tinyaudio | Output-only playback | — | MIT | yes | Smaller community |

**Crucial gotchas**: `realfft` does not normalize (divide by N or 1/√N) and produces N/2+1 complex bins; cpal `BufferSize::Default` on ALSA/PipeWire can resolve to anything up to `u32::MAX` (always pin Fixed, though Fixed on WASAPI only sets ring-buffer duration); aubio-rs / essentia wrappers are GPL-3.0 / AGPL-3.0 — unsafe for commercial closed-source; Symphonia is MPL-2.0 (file-level copyleft, OK for proprietary apps).

---

## 7. Rust ML Inference

| Crate | Backend | License | ONNX import | Mobile path | Real-world examples |
|---|---|---|---|---|---|
| **ort** (pykeio) | ONNX Runtime C++ | MIT / Apache-2.0 | yes (native) | iOS (CoreML EP) + Android (NNAPI / QNN EP); **must vendor a custom ONNX Runtime build** with `--minimal_build`, `--use_coreml`, `--use_nnapi` | Xybrid (LLM/ASR/TTS), `floriskappen/basic-pitch-rust` (proof-of-feasibility only) |
| **tract** (Sonos) | Pure-Rust kernels (CPU + Apple Metal GPU + CUDA + WASM) | MIT / Apache-2.0 | yes (also NNEF, TFLite, TF1) | yes (pure-Rust, no FFI) | Sonos production wake-word; no published Basic Pitch / CREPE example |
| **candle** (HuggingFace) | Pure Rust + optional CUDA / Metal / MKL / Accelerate | Apache-2.0 / MIT | **no** | iOS Metal works; Android works with caveats (open issues #1048, #2823, #3015) | Whisper, Llama, Mistral, Stable Diffusion ports |
| **burn** (tracel-ai) | Wgpu / CUDA / ROCm / Metal / Vulkan / LibTorch / Cpu | MIT / Apache-2.0 | yes (codegen) | mobile not first-class; open precision bug #4541 | Training + inference framework |
| **tch-rs** | libtorch C++ | MIT / Apache-2.0 | indirectly via torch.jit | **no documented mobile path** | Desktop ML research |
| **wonnx** | WebGPU via wgpu | MIT / Apache-2.0 | partial | theoretically yes (wgpu) | **Archived 2025-05-07** — do not use |

### Recommendation

- **Default**: `ort` 2.0 for `nmp.onnx` (Basic Pitch), with the `load-dynamic` feature for desktop iteration.
- **For mobile**: build ONNX Runtime from source with `--minimal_build` + the right execution providers; vendor the static lib per target. `download-binaries` does **not** ship iOS/Android binaries.
- **Watch tract** as the pure-Rust escape hatch if `ort` binary size or vendoring becomes painful, with a pre-commit op-coverage spike against `nmp.onnx`.
- **Avoid tch-rs** (no mobile), **avoid wonnx** (archived). Track candle and burn as 6-12 month re-evaluation candidates.

### Mobile path

```
Desktop (today)           iOS / Android (Phase 6)
┌────────────────┐        ┌──────────────────────────┐
│ Rust core      │        │ Rust core                │
│  └─ ort 2.0    │        │  └─ ort 2.0              │
│      (load-    │        │      (download-binaries  │
│       dynamic) │        │       feature OFF;       │
│  ← desktop ORT │        │       static-link a      │
│    binaries    │        │       custom ONNX RT     │
└────────────────┘        │       built with         │
                          │       --minimal_build,   │
                          │       --use_coreml or    │
                          │       --use_nnapi)       │
                          └──────────────────────────┘
```

Note: ort 2.x is still **release-candidate** (2.0.0-rc.12 as of March 2026); pin a specific rc and budget for one API migration before stable 2.0 ships.

---

## 8. Tauri Architecture & Real-Time Audio Pipeline

The well-supported pattern for a Tauri 2 app doing real-time pitch detection is a **3-thread architecture** with a wait-free SPSC ring at the audio-callback boundary and `tauri::ipc::Channel<T>` to the UI:

```
┌─────────────────────────────────────────────────────────────────┐
│                       OS Audio Subsystem                         │
│        (CoreAudio / WASAPI / ALSA-PipeWire / AAudio)             │
└─────────────────────────────────────────────────────────────────┘
                                 │ buffer (256–512 frames @ 48 kHz)
                                 ▼
┌─────────────────────────────────────────────────────────────────┐
│ cpal AUDIO CALLBACK THREAD (high priority / RT-promoted)         │
│   - copy samples (i16/u16/f32 → f32)                             │
│   - DC-block / pre-emphasis OPTIONAL HERE (better in worker)     │
│   - rtrb::Producer.push_slice(&samples)         ← NO alloc       │
│                                                  ← NO lock       │
│                                                  ← NO syscall    │
│                                                  ← NO println    │
│                                                  ← NO emit       │
└─────────────────────────────────────────────────────────────────┘
                                 │ wait-free SPSC ring (rtrb 0.3.4)
                                 │ (~3× largest analysis window)
                                 ▼
┌─────────────────────────────────────────────────────────────────┐
│ DSP WORKER THREAD (std::thread::spawn at app start)              │
│   - drain ring in window-sized chunks                            │
│   - DC blocker + pre-emphasis                                    │
│   - voice-activity gate (RMS + hangover)                         │
│   - YIN / MPM / Basic Pitch ONNX                                 │
│   - smoothing (200–500 ms running mean)                          │
│   - emit PitchUpdate via Channel::send(...)                      │
└─────────────────────────────────────────────────────────────────┘
                                 │ tauri::ipc::Channel<PitchUpdate>
                                 │ (ordered, fast, JSON-serialized)
                                 ▼
┌─────────────────────────────────────────────────────────────────┐
│ TAURI MAIN / RUNTIME THREAD                                      │
│   - serves #[tauri::command] entry points                        │
│   - holds CancellationToken in tauri::State                      │
└─────────────────────────────────────────────────────────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────┐
│ WEBVIEW (WKWebView / WebView2 / WebKitGTK)                       │
│   - JS Channel listener writes to a small ring (256 samples)     │
│   - requestAnimationFrame draws tuner needle / pitch ribbon      │
│   - state via Svelte runes (or Zustand for React)                │
└─────────────────────────────────────────────────────────────────┘
```

### 8.1 Streaming primitives

- **DO** use `tauri::ipc::Channel<T>` for all Rust→JS streams above ~10 Hz. Tauri's docs explicitly state events are "not designed for low latency or high throughput" and Tauri uses Channel internally for download progress, child-process output, WebSocket messages.
- **DO NOT** use `app.emit()` for pitch updates. Documented crash bug at high event rates (issue #8177, NVDA frameless-window regression #12901 still open as of mid-2026).

### 8.2 Cancellation pattern

```rust
use tokio_util::sync::CancellationToken;

// Stored in tauri::State at app start
struct AppState { cancel: CancellationToken }

#[tauri::command]
async fn analyze(state: tauri::State<'_, AppState>,
                 channel: tauri::ipc::Channel<ProgressEvent>)
    -> Result<(), String>
{
    let token = state.cancel.child_token();
    loop {
        if token.is_cancelled() { return Err("cancelled".into()); }
        let packet = format_reader.next_packet()?;        // packet boundary
        // ...decode + analyze...
        channel.send(ProgressEvent::Window { /* ... */ }).map_err(|e| e.to_string())?;
    }
}

#[tauri::command]
fn cancel(state: tauri::State<'_, AppState>) { state.cancel.cancel(); }
```

### 8.3 Latency budget (visual tuner, no through-monitoring)

| Stage | Time |
|---|---|
| Input device buffer (256 @ 48 kHz) | 5.3 ms |
| rtrb hand-off | < 1 ms |
| YIN / MPM analysis (2048 window) | 1–5 ms (single core) |
| Channel JSON serialize + cross IPC | ~1–3 ms |
| JS → rAF wait | up to 16.7 ms (60 Hz) |
| Render | up to 16.7 ms |
| **Total typical** | **~30–45 ms** |
| **Total worst case** | **~60–70 ms** |

For applications that need glass-to-glass under ~30 ms (live through-monitoring), tighten device buffer to 128 frames, raise to 120 Hz UI, and accept higher CPU.

### 8.4 What the audio callback must NOT do

No `Vec::push` / `Box::new` (allocation); no `Mutex` / `RwLock` / `parking_lot` lock; no `println!` / `log::*` (use a lock-free counter + main-thread aggregation); no `AppHandle::emit` / `Channel::send` (ring-buffer hand-off only); no file or network I/O.

### 8.5 Plugins to adopt early

`tauri-plugin-fs`, `tauri-plugin-store`, `tauri-plugin-dialog`, `tauri-plugin-log`, `tauri-plugin-window-state`, `tauri-plugin-updater`. macOS additionally needs `NSMicrophoneUsageDescription` in `Info.plist` and `com.apple.security.device.audio-input` entitlement under hardened-runtime + notarization.

---

## 9. Frontend UI

### 9.1 Framework

**SvelteKit 5 + adapter-static + Vite** is the recommended default for a fresh project: smallest webview payload, top-tier benchmark performance for non-canvas UI, official Tauri starter, runes pair well with imperative canvas code. **Caveats** (verifier finding):

- SvelteKit must run as static SPA (`adapter-static`, SSR disabled) inside Tauri — most SvelteKit features (server load, form actions, server endpoints) do not apply. Plain Svelte 5 + Vite is equally valid.
- Svelte 5 runes are only **partially** backwards-compatible with Svelte 4; budget time for runes idioms (whitespace, CSS scoping, prop declaration changes).
- "Smallest" / "fastest" superlatives are softer than the digest implied — SolidJS often beats Svelte on krausest js-framework-benchmark for runtime ops and bundle scaling beyond ~20 components.

If the team is React-deep, **React 19 + Vite + Zustand** is a fine alternative; the canvas hot path is imperative and framework-agnostic.

### 9.2 Notation library

| Library | Input | Output | License | Verdict |
|---|---|---|---|---|
| **OpenSheetMusicDisplay 1.9.x** | MusicXML / MXL | SVG | BSD-3-Clause | Default for MusicXML pipelines |
| VexFlow 5 | EasyScore / programmatic | Canvas / SVG | MIT | Use for short engraved snippets |
| abcjs 6.6.x | ABC notation | SVG + MIDI | MIT | If primary input is ABC |
| alphaTab 1.8.x | GuitarPro 3-7 / MusicXML / AlphaTex | SVG / Canvas + MIDI playback | **MPL-2.0** | Use only for guitar tablature |

OSMD does **not** localize note names (the underlying MusicXML `<step>` is permanently English A–G). Localized note rendering must happen in the app render layer (see §10). OSMD's only built-in localization hooks are `ChordSymbolLabelTexts` and `ChordAccidentalTexts`.

### 9.3 Spectrogram + waveform

**WaveSurfer.js 7.12.x + Spectrogram plugin** for any seekable file timeline + spectrogram. The plugin uses a Web Worker FFT, supports mel/bark/erb/log/linear scales, and pre-computes spectrogram data for large files via `frequenciesDataUrl`.

For real-time mic-input visualization only (no file seeking), AudioMotion-Analyzer is smaller and prettier — but is **AGPL-3.0-or-later** which is a hard-stop for closed-source distribution.

### 9.4 State management

- **Heavy real-time updates** (pitch samples at 50–100 Hz): **never** flow through framework reactivity. Push into refs / a small JS-side ring buffer; consume inside `requestAnimationFrame`.
- **UI shell state** (current piece, settings, transport): Svelte `$state` runes / Zustand for React / Solid signals.

### 9.5 UX patterns to adopt MVP

1. **Karaoke-style scrolling pitch ribbon** (SingStar / UltraStar Deluxe / Yousician-vocal pattern) — horizontal target bars + live frequency cursor + tolerance-window color fill. Scope to monophonic sung melody; do **not** rely on color alone (WCAG 1.4.1 — see §13).
2. **Centred-needle cents-deviation tuner** with green-when-centered chromatic readout — the universal mental model (Wikipedia: Electronic tuner; NTune-style green LED is precedented). Pair with a small history line (Korg / SoundCorset gauge+chart pattern) to surface vibrato / drift.
3. **Layered, time-aligned panes** for waveform + spectrogram + pitch contour (Sonic Visualiser + Vamp pattern). This is the architecture, not just the UI — each analyser is an independent module emitting time-aligned streams.
4. **Blob / piano-roll per-note view** with intra-note pitch curve overlay (Melodyne / Basic Pitch demo) — for transcription review.
5. **Tempo-independent slow-down with seamless A-B looping** (Capo / AnyTune / Sonic Visualiser variable-speed playback) for the offline workflow.

---

## 10. Music Theory & Note Math

This section is the spec for the core `frequency_to_note` function.

### 10.1 Reference

- **A4 reference is configurable**, default 440 Hz. Presets: 415 (Baroque), 430 (Classical / French historical), 435 (French diapason normal), 440 (ISO 16), 442/443 (continental orchestras / Berlin Philharmonic), 466 (Chorton). Optionally include 432 Hz for hobbyist demand without endorsing pseudoscience claims. Free-form entry recommended in **410–470 Hz** (matches Korg CA-50 industry norm; the digest's wider 400–500 was unjustified).
- **Octave numbering**: C4 = middle C = MIDI 60 (Scientific Pitch Notation / IPN / ASA). Document prominently. Optional UI toggle for users who prefer C3 = middle C (Yamaha/Roland convention). Internally always use MIDI numbers — note 60 is unambiguous.

### 10.2 Core formula

```rust
pub fn frequency_to_note(f_hz: f32, a4_hz: f32) -> NoteReading {
    // continuous MIDI number from A4 = 69
    let midi_continuous = 12.0 * (f_hz / a4_hz).log2() + 69.0;
    let midi = midi_continuous.round() as i32;
    let expected_hz = a4_hz * 2_f32.powf((midi - 69) as f32 / 12.0);
    let cents = 1200.0 * (f_hz / expected_hz).log2();
    NoteReading { midi, expected_hz, cents }
}
```

Both `cents = 100 * (midi_continuous - midi)` and `cents = 1200 * log2(f / expected_hz)` are numerically equivalent at IEEE 754 double precision. Choose the latter for testability.

### 10.3 Tolerances

| Band | Cents | Meaning |
|---|---|---|
| Tight (green) | ±5 | At trained-listener JND; harder than mass-market hardware achieves |
| Medium | ±10–15 | Reliably perceptible only to trained ears |
| Loose | ±20 | Casual-listening "in tune" |
| Bin edge | ±50 | Nearest-note assignment cutoff |

Display a smoothed running mean of cents deviation over **200–500 ms**, not instantaneous frame-by-frame readout. Listeners perceive the mean as pitch center; vibrato (5–7 Hz, ±50 cents typical) will saturate any ±5-cent display without smoothing.

### 10.4 Instrument / voice ranges (Bayesian priors)

| Profile | f_min | f_max |
|---|---|---|
| Bass guitar (4-string) | 35 Hz (E1) | 500 Hz |
| Guitar (6-string standard) | 80 Hz (E2) | 1300 Hz |
| Piano | 27 Hz (A0) | 4200 Hz (C8) |
| Violin | 196 Hz (G3) | 2700 Hz |
| Voice — bass | 70 Hz | 350 Hz |
| Voice — soprano | 250 Hz | 1600 Hz |
| Generic | 60 Hz | 2000 Hz |

Hard-clamp the YIN τ search range to the chosen profile — single highest-leverage octave-error mitigation.

### 10.5 Tuning systems

12-TET default. Architect the math to take a **Tuning** parameter from day one even if only 12-TET ships first; later modes (Just, Pythagorean, 19-TET, 31-TET, custom cents-per-semitone table) become a setting, not a refactor. Equal-temperament major thirds are 14 cents sharp of just (audible); choirs and string quartets singing "in tune" will register as flat, so add a note in advanced settings.

### 10.6 MIDI export math

For per-note pitch bend matching Basic Pitch convention: ±2 semitones range, `PITCH_BEND_SCALE = 4096` ticks/semitone, 14-bit value centered at 8192. Emit `RPN 0` (Pitch Bend Sensitivity) MSB=2, LSB=0 at track start so receivers do not default to ±12 or other values.

---

## 11. Real-Time Latency & Buffer Strategy

### 11.1 Recommended defaults

| Mode | Sample rate | Window | Hop | Frames/s | f_min | Notes |
|---|---|---|---|---|---|---|
| Live tuner (default) | 48 kHz | 2048 | 512 | 93.75 | ~47 Hz | Fits guitar low E2; ~21 Hz update period exceeds flicker fusion |
| Live tuner (bass / low voice) | 48 kHz | 4096 | 1024 | 46.9 | ~24 Hz | Required for bass guitar low E1 |
| Ear-training drill | 44.1/48 kHz | 4096 | 1024 | ~47 | ~24 Hz | Sustained-note mode; latency tolerable |

(Verifier corrected: 48 kHz / 512 hop = 93.75 fps, not 86. Flicker-fusion threshold is ~50–80 Hz, so the chosen rate **exceeds** fusion comfortably.)

### 11.2 Ring buffer pattern

```rust
// At stream start
let (mut prod, cons) = rtrb::RingBuffer::<f32>::new(window_size * 3);

// In cpal callback: push_slice(&data)
// In worker thread: read_chunk(window_size).copy_to_slice(&mut buf)
```

Size at **3× the largest analysis window** (e.g., 8192 / 16384 f32 samples for 2048 / 4096 windows) so one missed scheduler wakeup does not drop samples.

### 11.3 End-to-end latency budget

| Stage | Typical | Worst case |
|---|---|---|
| Mic ADC + driver | 1–3 ms | 5 ms |
| Device buffer (256 @ 48 kHz) | 5.3 ms | 5.3 ms |
| Ring hand-off | < 1 ms | 1 ms |
| DC block + pre-emphasis | < 0.1 ms | 0.5 ms |
| YIN / MPM (2048 window) | 1–5 ms | 8 ms |
| Smoothing | 200–500 ms intentional | — |
| Channel + JSON | 1–3 ms | 5 ms |
| JS rAF wait | 8 ms (avg) | 16.7 ms |
| Canvas draw | 5–10 ms | 16.7 ms |
| **Mic → screen total** | **~40 ms** | **~60–70 ms** |

For Linux PipeWire users wanting tighter latency: `pw-metadata -n settings 0 clock.force-quantum 256` reduces quantum to ~5.3 ms (default 1024 = ~21 ms).

---

## 12. Testing Strategy & Datasets

### 12.1 Test pyramid

```
                           ┌──────────────────────────┐
                           │ Tier 4: Full benchmarks  │  ← nightly / on-demand
                           │   MAESTRO (101 GB)       │     (cargo bench / make bench)
                           │   MUSDB18-HQ (22.7 GB)   │
                           └──────────────────────────┘
                       ┌────────────────────────────────┐
                       │ Tier 3: Small dataset slices   │  ← gated CI
                       │   Bach10, MDB-stem-synth subset│     (cargo test --features dataset)
                       │   GuitarSet (CC BY 4.0, ~8 GB) │
                       └────────────────────────────────┘
                  ┌──────────────────────────────────────┐
                  │ Tier 2: Philharmonia single-note     │  ← every PR
                  │   fixtures (~50–100 files in repo)   │
                  └──────────────────────────────────────┘
              ┌──────────────────────────────────────────┐
              │ Tier 1: Synthesized signals (in-test)    │  ← every PR (cargo test)
              │   sine, two-tone, vibrato, noise         │     ms-fast, deterministic
              └──────────────────────────────────────────┘
```

Use `#[ignore]` + `cargo test -- --ignored` (or a separate `tests/` integration binary) for slow tiers. Cargo features are for genuinely optional code, not for skipping tests — `cargo test` runs only default features by default, so feature-gated tests can be silently skipped.

### 12.2 Datasets

| Dataset | Size | License | Best for |
|---|---|---|---|
| **MAESTRO v3** | ~199h, 101 GB | CC BY-NC-SA 4.0 | Piano transcription benchmark (Tier 4) |
| **MDB-stem-synth** | 230 stems, 1.8 GB | CC BY-NC 4.0 | Frame-level F0 ground truth |
| **GuitarSet** | 360 clips, 8.2 GB | **CC BY 4.0** | Per-string F0 + transcription (most permissive) |
| **MUSDB18-HQ** | 150 tracks, 22.7 GB | Academic-only | 4-stem separation benchmark |
| **Slakh2100** | 145h | Academic / non-commercial in practice | Synthesized multi-track |
| **MusicNet** | 330 tracks, 11.1 GB | CC BY 4.0 | Classical multi-instrument; ~4% label noise |
| **MedleyDB** | 122+74 multitracks | CC BY-NC-SA per track | Melody F0, instrument activations |
| **Bach10** | 10 pieces | unspecified, research-only | Multi-pitch sanity baseline |
| **Philharmonia** | per-instrument samples | Free reuse incl. commercial; not as a sample pack | Single-note unit-test fixtures |

### 12.3 Metrics

Wrap **mir_eval** (MIT, v0.8.2 Feb 2025) and **museval** / `sigsep-mus-eval` (BSSEval v4) via PyO3 for the offline / CI eval harness rather than reimplementing in Rust. Reimplementing risks subtle off-by-one errors in tolerance windows, voicing logic, and BSSEval permutation that would make your numbers non-comparable to MIREX submissions. Use PyO3 only at the test boundary — not at runtime.

For native Rust live-debug overlays, reimplement only frame-level F0 cents error, which is trivial.

### 12.4 Property-based tests (proptest)

Generate `(freq, sample_rate, window_size, snr)` tuples within the algorithm's stated operating range and assert tolerance bounds. Always include adversarial fixtures: silence, white noise, two-tone, sub-octave-strong tones, missing fundamental.

```rust
#[test]
fn yin_440hz_clean_within_5_cents() {
    let signal = sine_wave(440.0, 48000.0, 2048);
    let f0 = yin(&signal, 48000.0).unwrap();
    let cents = 1200.0 * (f0 / 440.0).log2();
    assert!(cents.abs() < 5.0, "expected ±5 cents, got {cents}");
}
```

---

## 13. Prior Art & UX Patterns to Borrow

Cleanest exemplars: **Sonic Visualiser** (GPL-2.0+, layered-pane + Vamp plugin architecture, v5.2.1 Mar 2025), **Spotify Basic Pitch** (basicpitch.io browser demo), **Tartini** (open-source MPM tuner; sites unreachable, project effectively abandoned but algorithm influential), **Audacity** (Qt v4 in development), **Demucs/Spleeter** (MIT separation), and the closed-source **Melodyne / iZotope RX / Yousician / SingStar / Capo / AnyTune / SoundCorset**.

Five concrete patterns to adopt MVP:

1. **Karaoke pitch ribbon** for monophonic sung-melody modes (SingStar / UltraStar Deluxe).
2. **Centred-needle cents tuner** with chromatic note label and green in-tune cue + scrolling history (Korg + SoundCorset gauge+chart).
3. **Layered time-aligned analysis panes** (Sonic Visualiser) — architectural pattern for adding analysers without UI rewrites.
4. **Blob + intra-note pitch curve** for polyphonic-transcription view (Melodyne) once Basic Pitch is integrated.
5. **A-B looping with pitch-preserving slow-down** (Capo / AnyTune) for offline practice.

---

## 14. Risks, Open Questions, and Decisions Deferred

### 14.1 Verifier-corrected positions (from the verifications payload)

These are recommendations the digest made that the verification phase refuted or partially-corrected. The body of this report reflects the corrected position; this section calls them out so the team is aware.

| Original digest claim | Verdict | Corrected position |
|---|---|---|
| MPM/YIN 1024 window covers low instruments; cycfi/q implements MPM/YIN; alesgenova "active" | partially | 2048 min for guitar E2; 4096 for bass E1. cycfi/q ships its own BACF/"Hz" detector (BACF retired in v1.5) — not MPM/YIN. alesgenova stagnant since June 2022. |
| marl/crepe "frozen since 2018"; CREPE unconditionally noise-robust | partially | marl/crepe 0.0.16 was published Aug 2024 — sporadic, not frozen. CREPE noise-robustness is contested in cross-talk / narrowband (Kato & Kinnunen 2019). |
| Basic Pitch the "only" polyphonic AMT model meeting all six criteria; ort 2.0 settled | partially | Soften to "only widely-deployed." ort 2.0 is still rc.12. `floriskappen/basic-pitch-rust` is an unlicensed 10-commit proof-of-feasibility, not a base to fork. Track Basic Pitch issues #190 (frame-drift), #164 (tflite packaging). |
| O&F "strongest piano-only model"; jongwook repo "archived" | partially | Kong et al. 2020 (MIT) reports Onset F1 96.72 > O&F 95.32 on MAESTRO V1.0.0 — stronger tier-2 target. `jongwook/onsets-and-frames` is inactive (last commit Jul 2021), not formally archived. |
| Demucs v4 "best balance" of quality + license + simplicity | partially | Quality ceiling now belongs to BS/Mel-RoFormer. facebookresearch/demucs archived 2025-01-01; adefossez/demucs maintenance-only; PyPI 4.0.1 is from Sep 2023. ~1.5× CPU realtime is README-anecdotal. No first-party ONNX/CoreML. |
| RoFormer doesn't separate 'other' well; combine with HTDemucs for 'other' | partially | BS-RoFormer 'other' 7.44 dB > HTDemucs 5.74–5.85 dB. Both struggle, but HTDemucs is not a better fallback specifically for 'other'. |
| Symphonia decodes only WAV/FLAC/MP3/OGG; cpal "actively maintained" | partially | Symphonia ALSO decodes AAC-LC, ALAC, MP4/M4A, AIFF, CAF — but only with `all-codecs`/`all-formats` features explicitly enabled. cpal sole maintainer publicly seeks handoff (issue #981). |
| `BufferSize::Fixed` "guarantees latency on all backends" | partially | Fixed is a request, not a guarantee. On WASAPI, Fixed sets ring-buffer duration only; callback period = `GetDevicePeriod()`. Some Windows/ALSA configs reject Fixed outright (#544, #534, #1214). Always query the supported range and clamp. |
| SvelteKit "smallest payload, top-tier perf"; OSMD "only" MusicXML library | partially | SolidJS often beats Svelte on krausest benchmark; SvelteKit must run as static SPA inside Tauri (negates server features); Svelte 5 runes have a real migration cost. Verovio (LGPL-3.0) is the other first-class MusicXML renderer; OSMD's edge is the BSD-3 license + built-in Cursor API. |
| f_min = 70 Hz covers bass low E; live update-rate "matches flicker fusion" | partially | Bass guitar low E is E1 ≈ 41 Hz, not E2 — 70 Hz floor fails for bass. 48 kHz / 512 hop = 93.75 fps (not 86) — **exceeds** flicker fusion (~50–80 Hz), does not match it. |
| Ear-training f_min = 55 Hz at 4096 window robust | partially | 4096 ≈ 5.1 periods of 55 Hz — lower end of robust autocorrelation. For noisy laptop mics, use 8192 (~186 ms). Frequency-resolution benefit only applies to time-domain methods or FFT with sub-bin interpolation. |

### 14.2 Weak-evidence claims worth revisiting

Several digest tracks were flagged with weak evidence — quote with caution:

- "CREPE outperforms pYIN at the 50-cent threshold on MDB-stem-synth and noisy versions" (CREPE README — verify against original PDF tables before publishing).
- ONNX Runtime `--minimal_build` binary-size targets (Microsoft does not publish a single canonical KB number).
- Whether Spotify Basic Pitch's pretrained ICASSP 2022 weights file is intended to be Apache-2.0 specifically (the repo LICENSE covers it by convention; an explicit statement in the model card is absent).
- Whether `sonos/tract` actually loads `nmp.onnx` end-to-end without op-coverage gaps — a 1-day spike is recommended before committing to tract over ort.
- Whether NNAPI actually accelerates Basic Pitch's CNN ops on common Snapdragon / Tensor / Exynos SoCs vs falling back to CPU.

### 14.3 Items requiring user / business input

- **Commercial vs free distribution**. Drives whether MUSDB-trained Demucs / RoFormer weights are usable, whether AGPL deps (AudioMotion-Analyzer) are acceptable, and whether the EU AI Act Article 53 transparency obligations cascade.
- **Mobile timeline**. If iOS is required within 12 months, the v2 mobile DX gaps need budgeting; if not, defer.
- **Through-monitoring**. If the user must hear themselves through the app (not just see a needle), sub-10 ms round trip is needed and the buffer/algorithm choices shrink accordingly.
- **Polyphonic priority**. Phase 3 timing depends on whether ear-training is the immediate driver (monophonic only, defer polyphonic) or song-analysis is (advance polyphonic).
- **Default note-name convention**. English C/D/E vs German H, fixed-do vs movable-do, Sargam — drives both i18n architecture and ear-training pedagogy.
- **Outside legal counsel sign-off** before any commercial release that bundles MUSDB-derived weights.

---

## 15. Phased Implementation Roadmap

### Phase 0 — Project Skeleton & TDD Harness

**Deliverables**: Cargo workspace (`core/`, `tauri/`, `tests/`, `fixtures/`); GitHub Actions CI matrix `ubuntu-latest`/`macos-14`/`windows-2022` running `cargo fmt --check`, `clippy -D warnings`, `cargo test`; License Register at `/licenses/REGISTER.md` (one row per dep including pretrained weights); skeleton Tauri shell with the standard plugins; `frequency_to_note(f, a4)` with property tests + golden tests at A0/C4/A4/C8 and ±50 cent edges.

**Acceptance**: CI green on all three OS runners; `frequency_to_note` golden tests cover MIDI 0–127 with errors < 0.001 cents; License Register reviewed.

**Risks**: CI runner version drift (pin specific images); Tauri RC plugin instability.

### Phase 1 — Live Monophonic Tuner

**Deliverables**: cpal capture stream at 48 kHz with `BufferSize::Fixed(256)` and `realtime` feature where available; 3-thread architecture (callback → rtrb 0.3.4 → DSP worker → `tauri::ipc::Channel<PitchUpdate>`); YIN and MPM (vendor `pitch-detection` 0.3.0) with instrument-range priors (Guitar / Bass / Voice-low / Voice-high / Generic / Piano); 200–500 ms running-mean cents smoothing; voice-activity gate (RMS + 5-frame hangover); tuner UI (centred-needle blue/orange + shape, big chromatic letter, history line, A4 selector 415/430/435/440/442/443/466 + free-form 410–470); macOS `NSMicrophoneUsageDescription` + audio-input entitlement.

**Acceptance**: mic-to-screen < 50 ms typical (M-series macOS, Linux PipeWire quantum 256, Windows WASAPI shared); ±5 cents on synthesized 440 Hz sine; ±20 cents on Philharmonia fixtures at SNR > 30 dB; UI does not freeze on mid-stream device disconnect.

**Dependencies**: Phase 0.

**Risks**: `BufferSize::Fixed` rejected on some Windows configs (#544, #534) — clamp-to-supported-range fallback required.

### Phase 2 — Record + Playback + Offline Note Display

**Deliverables**: record mic to FLAC/WAV via Symphonia + `hound`; per-recording metadata schema (sample rate, A4, instrument profile) in SQLite via rusqlite 0.40; WaveSurfer.js 7 timeline with playhead, seek, A-B loop; offline pitch contour overlay (recompute pYIN, cache); tempo-independent slow-down (rubato + phase vocoder); export to JAMS.

**Acceptance**: hour-long recordings load with < 500 MB resident; cancellation via `CancellationToken` returns within one packet boundary (~20–40 ms); re-opening a previously-analysed file < 1 s (cache hit).

**Dependencies**: Phase 1.

**Risks**: SQLite schema migrations (start with explicit version column); Symphonia M4A correctness issues (#480, #475) — graceful "unsupported format" degrade.

### Phase 3 — File Upload + Offline Polyphonic Transcription via Basic Pitch

**Deliverables**: file-import dialog (WAV/FLAC/MP3/OGG/AAC-LC/ALAC/M4A); internal resample to 22.05 kHz (rubato); Basic Pitch ONNX inference via `ort` 2.0 with streaming windows (`AUDIO_N_SAMPLES=43844`, 30-frame overlap, half-overlap trim); note events with per-note pitch bend (channel-per-note routing); MIDI export via `midly` 0.5.3 with `RPN 0` ±2 at track start; optional MusicXML export via `hedgetechllc/musicxml`; OSMD-rendered preview with per-note ARIA labels; Canvas2D piano-roll with intra-note pitch-curve overlay.

**Acceptance**: 7m45s reference file within ±5% RPA of Spotify Python reference; peak memory < 1 GB on hour-long files; MIDI round-trips through MuseScore 4 without lost pitch bends.

**Dependencies**: Phase 0–2.

**Risks**: ort 2.0 RC API churn; Basic Pitch issue #190 (frame drift) — add temporal-alignment regression test; CoreML / NNAPI EP op-coverage gaps may force CPU fallback on mobile.

### Phase 4 — Stem Separation + Per-Track Notes

**Deliverables**: Demucs v4 (`htdemucs`) ONNX via `ort` with exact triangular-window overlap-add from `apply.py` (segment 7.8 s, overlap 0.25, `transition_power=1.0`); 4-stem output as FLAC; per-stem Basic Pitch (skip drums); layered piano-roll with per-stem color + solo/mute; "other" labelled as residual; LRU disk-cache eviction; license-aware UX (bundled Demucs for free tier; paid tier requires user-supplied or commercial-licensed weights).

**Acceptance**: 4-stem MUSDB18-HQ test within 1 dB SDR of reference Demucs v4; hour-long song under 20 min on M3 Pro CPU / 5 min on RTX 4070 GPU; mid-segment cancellation works.

**Dependencies**: Phase 0–3, **outside-counsel review** of weight licenses.

**Risks**: weight-license ambiguity (§14); HTDemucs degradation at 96 kHz (downsample); community ONNX exports vary in fidelity — pin a specific export with checksum.

### Phase 5 — Ear-Training Games

**Deliverables**: interval recognition, chord-quality ID, melody dictation, scale/mode ID (monophonic only at MVP); spaced-repetition scheduling (FSRS or SM-2); movable-do / fixed-do / English / German / Sargam note-name systems; bundled SoundFont (FluidR3 GM or GeneralUser GS) for prompt playback via `oxisynth`; sonification mode for visually-impaired users (drone + beat-rate panning + TTS); per-exercise A4 reference.

**Acceptance**: interval scoring within 1% of reference EarMaster; movable-do syllables correct under modulation (chromatic alterations Di/Ri/Fi/Si/Li and Ra/Me/Se/Le/Te); sonification cues independently toggleable.

**Dependencies**: Phase 1 (input), Phase 2 (retake comparison).

**Risks**: pedagogy is a real product domain — engage a music educator before locking exercise design. Verify SoundFont license terms.

### Phase 6 — Mobile (iOS + Android)

**Deliverables**: Tauri Mobile builds (iOS arm64 + simulator; Android arm64-v8a + armeabi-v7a); custom ONNX Runtime per target (`--minimal_build`, `--use_coreml`, `--use_nnapi`), statically linked; Tauri mobile plugin bridging to AVAudioFile (iOS) and MediaExtractor + MediaCodec (Android) for HE-AAC / xHE-AAC / hardware-accelerated AAC; `AVAudioSession` category management; Android Oboe / AAudio backend; permission flows; App Store / Play Store privacy nutrition labels.

**Acceptance**: < 60 ms mic-to-screen on iPhone 14 Pro; < 15% battery drain over 30 min continuous tuning; first-submission acceptance by both stores.

**Dependencies**: Phase 1 stable, Phase 3 validated, **outside legal review** of model weights for store distribution.

**Risks**: Tauri Mobile DX gaps (flagged by maintainers); NNAPI may not accelerate Basic Pitch (CPU-only mobile inference may be impractical for hour-long files); AAC patent royalties (Via LA ~$0.98/unit) for shrink-wrap commercial apps at scale.

---

## 16. References

Sources cited in the research/verification payload, grouped by track.

**Pitch detection (monophonic)** — de Cheveigné & Kawahara, "YIN" (JASA 2002); Mauch & Dixon, "pYIN" (ICASSP 2014); Kim et al., "CREPE" (ICASSP 2018, arXiv:1802.06182); Gfeller et al., "SPICE" (TASLP 2020, arXiv:1910.11664); McLeod & Wyvill, "A Smarter Way to Find Pitch" (ICMC 2005); Kato & Kinnunen, IEEE/ACM TASLP 2019 (CREPE noise-robustness critique); librosa (github.com/librosa/librosa); aubio (aubio.org); alesgenova/pitch-detection; Sytronik/pyin-rs; maxrmorrison/torchcrepe; Cycfi Q (github.com/cycfi/q); Wikipedia: Pitch detection algorithm.

**Polyphonic transcription** — Bittner et al., Basic Pitch (arXiv:2203.09893, ICASSP 2022); Hawthorne et al., Onsets and Frames (arXiv:1710.11153, arXiv:1810.12247); Hawthorne et al., Seq2Seq Piano Transcription (arXiv:2107.09142); Gardner et al., MT3 (arXiv:2111.03017); Kong et al., High-resolution Piano Transcription with Pedals (arXiv:2010.01815); github.com/spotify/basic-pitch; github.com/magenta/mt3; github.com/jongwook/onsets-and-frames; Omnizart.

**Source separation** — Défossez, Hybrid Demucs (arXiv:2111.03600); Rouard, Massa, Défossez, HT Demucs (arXiv:2211.08553); Lu et al., BS-RoFormer (arXiv:2309.02612); Luo & Yu, BSRNN (arXiv:2209.15174); github.com/facebookresearch/demucs; github.com/adefossez/demucs; github.com/lucidrains/BS-RoFormer; github.com/ZFTurbo/Music-Source-Separation-Training; github.com/sigsep/open-unmix-pytorch; github.com/deezer/spleeter; github.com/kuielab/mdx-net; sigsep.github.io/datasets/musdb.html.

**Rust DSP / audio I/O / ML** — github.com/RustAudio/cpal; github.com/pdeljanov/Symphonia; HEnquist/realfft + ejmahler/RustFFT + HEnquist/rubato; mgeier/rtrb + agerasev/ringbuf; github.com/pykeio/ort + ort.pyke.io; github.com/sonos/tract; github.com/huggingface/candle; github.com/tracel-ai/burn; github.com/LaurentMazare/tch-rs; github.com/webonnx/wonnx (archived); onnxruntime.ai.

**Tauri + frontend** — v2.tauri.app + github.com/tauri-apps/tauri; OpenSheetMusicDisplay; VexFlow (0xfe and vexflow orgs); abcjs; alphaTab; wavesurfer.js; AudioMotion-Analyzer; Tone.js; html-midi-player.

**Testing / datasets** — github.com/mir-evaluation/mir_eval; github.com/sigsep/sigsep-mus-eval; MAESTRO (magenta.tensorflow.org/datasets/maestro); MDB-stem-synth (zenodo.org/record/1481172); GuitarSet (zenodo.org/record/3371780); MUSDB18; MusicNet (zenodo.org/records/5120004); MedleyDB; Slakh2100 (github.com/ethman/slakh-utils); Philharmonia samples (philharmonia.co.uk/resources/sound-samples/).

**Standards, theory, accessibility, i18n** — ISO 16:1975; WCAG 2.1 (w3.org/TR/WCAG21); W3C ARIA APG; github.com/AccessKit/accesskit; github.com/dequelabs/axe-core; Project Fluent (github.com/projectfluent/fluent.js, fluent-rs); github.com/unicode-org/icu4x; github.com/format-js/formatjs; smufl.org/fonts; Wikipedia: A440, Concert pitch, Cent (music), Equal temperament, Just intonation, Missing fundamental, Formant, Vocal range, Sonification, Vibrato.

**Licensing** — Apache License 2.0 (apache.org/licenses/LICENSE-2.0); Creative Commons on AI training (creativecommons.org/2021/03/04/...); Via LA AAC licensing (via-la.com/licensing-2/aac/); Spotify Basic Pitch NOTICE (raw.githubusercontent.com/spotify/basic-pitch/main/NOTICE).
