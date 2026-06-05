# NeuralPitch — Architecture and Design

## Status

**Active.** This is the canonical architecture and design document for the NeuralPitch project, locked at the 2026-06-02 design interview. It is the single source of truth for project shape, dependency selection, module layout, real-time pipeline topology, persistence, concurrency, testing, observability, and roadmap. Conflicts between this document and any other artefact in the repository are resolved in favour of this document; updates land via pull request and are recorded in the ADR index (§17).

This document supersedes the three upstream research reports as the _decision_ surface; those reports remain authoritative as the _evidence_ surface and are cited by relative path throughout. The reports are:

- [`../research/RESEARCH-REPORT.md`](../research/RESEARCH-REPORT.md) — DSP, ML, and ecosystem survey.
- [`../research/MODULAR-PITCH-RESEARCH.md`](../research/MODULAR-PITCH-RESEARCH.md) — modular pitch-pipeline research, trait surface, per-stem dispatch.
- [`../research/REPO-CONVENTIONS-REPORT.md`](../research/REPO-CONVENTIONS-REPORT.md) — repository conventions, idiomatic Rust, hygiene baseline.

This file lives at `docs/design/DESIGN.md`; the `../research/...` paths above resolve relative to this file's directory.

## 1. Project Context, Scope, Personas

### 1.1 Project description

`neural-pitch` is a desktop-first, mobile-aspirational pitch-detection and ear-training application built on Tauri 2.x with a pure-Rust core. Its primary subject is the human singing voice: the app listens to a microphone (or imported audio later), tells the user what notes are being sung, and supports the user in learning the boundaries and intonation of their own range. It is a free and open-source software (FOSS) personal-and-learning project distributed under the Rust-ecosystem-standard dual `MIT OR Apache-2.0` license (see ADR-0001), developed in the open as a vehicle for both a usable application and the author's own study of real-time DSP, neural pitch detection, and Rust audio engineering. The architecture is deliberately modular so that future capabilities — whole-song transcription, stem separation, ear-training games, and mobile (iOS / Android) builds via Tauri Mobile — slot in as additive phases without disturbing the live tuner. Grounding for this framing: [`../research/RESEARCH-REPORT.md`](../research/RESEARCH-REPORT.md) §1 and [`../research/MODULAR-PITCH-RESEARCH.md`](../research/MODULAR-PITCH-RESEARCH.md) §1.

### 1.2 Personas

The project explicitly designs for a small, well-known audience. Anonymous-internet-stranger ergonomics, growth funnels, and engagement loops are not in scope.

| Persona                                | Role                                                               | Skill assumption                                                | What they need from the app                                                                                                                      |
| -------------------------------------- | ------------------------------------------------------------------ | --------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Self (primary)**                     | Developer-author; singer learning their own voice                  | Comfortable in Rust, audio DSP, terminal; intermediate musician | Trustworthy live pitch readout for vocal practice; vocal range and intonation feedback; a sandbox to learn neural pitch detection by building it |
| **Friends / contributors (secondary)** | Casual testers; a handful of musicians the author knows personally | Mixed: one-click installs expected, but tolerant of rough edges | Install on macOS / Linux / Windows; sing into it; report bugs in plain language; optionally clone, build, and contribute                         |

Implication for the design: the user-experience bar is "would I personally use this every day, and would I be comfortable handing the binary to a friend who sings". It is **not** "would a stranger discovering this on a store page have a frictionless onboarding". This authorises certain low-cost simplifications that a commercial product could not make — for example, defaulting to visual-only feedback with no through-monitoring (Phase 1, see ADR-0006), shipping recordings into a single app-managed library directory (ADR-0012), and deferring localisation infrastructure beyond a single English locale source — all with the architectural seams in place to relax these assumptions later.

### 1.3 In scope (by phase)

The phased roadmap from §13 is the canonical scope statement; this section reproduces only the per-phase headline so that downstream sections can reference it.

| Phase | Headline scope                                                                                                            |
| ----- | ------------------------------------------------------------------------------------------------------------------------- |
| 0     | Project skeleton, TDD harness, CI, golden tables — no Tauri UI                                                            |
| 1     | Live monophonic tuner; YIN / MPM with auto-prior; visual-only feedback; no recording; no neural                           |
| 2     | Recording + playback; offline pYIN; PESTO ONNX neural backend behind `feature = "neural"`; vocal range; vibrato detection |
| 3     | File upload; whole-mix Basic Pitch polyphonic transcription; MIDI export                                                  |
| 4     | Ear-training games (movable-do solfege; Smule-style karaoke pitch ribbon)                                                 |
| 5     | Stem separation (HTDemucs default, BS-RoFormer additive when GPU detected) and per-stem detector dispatch                 |
| 6     | Mobile iOS + Android via Tauri Mobile                                                                                     |
| 7     | Optional learning side-quests (pure-Rust PESTO via burn / candle; fine-tunes; OSS contributions)                          |

The phase ordering — ear-training before stem separation — is locked by ADR-0009.

### 1.4 Explicit non-goals

The following are intentionally out of scope. Each non-goal is a deliberate negative constraint that excludes feature work, defends design simplicity, and signals to contributors what kinds of pull requests will not be accepted.

| Non-goal                                          | Rationale                                                                                                                                                                                                                                                                                                                                                |
| ------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **Not a recording studio / DAW**                  | No multi-track timeline editing, no plug-in hosting, no mixing, no mastering, no MIDI sequencing. The recording feature (Phase 2) exists only to capture single-take vocal performances for analysis and review.                                                                                                                                         |
| **Not a real-time performance / monitoring tool** | Phase 1 is **visual-only feedback, no through-monitoring** (ADR-0006). No latency-critical headphone mix; no live-on-stage use case. A future `MonitoringPipeline` is allowed as an additive sibling, but the Phase 1 latency budget (see §13) is sized for visual reaction, not audio reinjection.                                                      |
| **Not a commercial product**                      | No telemetry, no crash reports, no analytics, no ads, no in-app purchases, no accounts, no cloud sync. All behaviour is local-only; the only network calls are explicit user-initiated model downloads (Phase 2+). License-encumbered model weights are correspondingly only a concern at the level of redistributing the FOSS app, not commercial sale. |
| **Not a social / community platform**             | No accounts, no sharing, no leaderboards, no song library exchange, no user-generated-content store. Recordings are stored in a per-user app-managed library on the local filesystem and never leave the device unless the user manually exports a file.                                                                                                 |
| **Not a generic audio-analysis framework**        | The Rust core is reusable (designed with no Tauri imports so it can later back a CLI or mobile shell), but the public surface is shaped by the singing-voice primary use case. Generality across arbitrary instruments is a Phase 1 advanced setting and a Phase 5 stem-separation outcome — not a v0.1 design driver.                                   |

### 1.5 Anchoring quotes from the research

```text
RESEARCH-REPORT.md §1
"neural-pitch is a desktop-first, mobile-aspirational app that listens to or imports
audio and shows what notes are being played or sung. ... The Rust core is built
standalone (no Tauri imports) so it can later be linked into mobile shells or a CLI.
... Use-case priorities: ear training / sight-singing first; stem separation +
per-track notes second."
```

```text
MODULAR-PITCH-RESEARCH.md §1
"Manual instrument selector should not be the default UX. ... Build a modular
PitchEstimator trait so backends are swappable, and switch the default to auto-detect
instead of forcing the user to pick Guitar / Bass / Voice."
```

These two quotes pin the singing-voice primary use case and the auto-by-default UX posture that the rest of this document elaborates.

## 2. Architectural Principles

This section enumerates the load-bearing principles that govern every subsequent design decision. Each principle is numbered (P1–P10) and cited by number in later sections. When two principles conflict, the lower-numbered principle wins unless the conflict is explicitly called out and resolved in writing.

The principles are derived from three upstream sources:

| Source                                                      | Path                                                                               |
| ----------------------------------------------------------- | ---------------------------------------------------------------------------------- |
| Research report (DSP, ML, ecosystem survey)                 | [`../research/RESEARCH-REPORT.md`](../research/RESEARCH-REPORT.md)                 |
| Modular pitch-pipeline research (trait shape, backend menu) | [`../research/MODULAR-PITCH-RESEARCH.md`](../research/MODULAR-PITCH-RESEARCH.md)   |
| Repo conventions (idiom, hygiene, FOSS norms)               | [`../research/REPO-CONVENTIONS-REPORT.md`](../research/REPO-CONVENTIONS-REPORT.md) |

### 2.1 Summary table

| #   | Principle                                              | One-line consequence                                                                                               |
| --- | ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------ |
| P1  | Modular trait boundaries before vertical features      | Backends sit behind traits before any concrete backend is written.                                                 |
| P2  | Pure-Rust core, Tauri only at the shell boundary       | `neural-pitch-core` has zero `tauri::*` imports.                                                                   |
| P3  | The audio callback is sacred                           | No `alloc`, no lock, no syscall, no panic in the cpal callback.                                                    |
| P4  | Tests before code (TDD)                                | Every public function lands with its failing test in the same series.                                              |
| P5  | Idiomatic Rust + Linux-kernel commit hygiene           | `clippy::pedantic`, `unsafe_code = "forbid"`, kernel-style commits, DCO.                                           |
| P6  | FOSS-first, no telemetry, local-only data              | Dual MIT OR Apache-2.0; zero outbound traffic except user-initiated model downloads.                               |
| P7  | Auto-detect over user-config                           | The singing primary use case must Just Work without an instrument selector.                                        |
| P8  | Ship visual-only Phase 1, modular for monitoring later | No through-monitoring path day 1; `MonitoringPipeline` reserved as additive sibling.                               |
| P9  | Bake mobile-shape day 1, defer mobile builds           | `crate-type = ["staticlib","cdylib","rlib"]`, lib name `neural_pitch_lib`, no actual mobile targets until Phase 6. |
| P10 | Fail loudly in tests, gracefully in production         | `unwrap`/`expect` denied in production code; tests exempt; production paths return `Result`.                       |

### 2.2 P1 — Modular trait boundaries before vertical features

The pitch-detection surface is defined as a `PitchEstimator` trait before the first concrete estimator is written (ADR-0007). Backends are added behind Cargo features; the trait shape is fixed in Phase 0 even though only YIN/MPM ship in Phase 1. Rationale and trait-surface options are surveyed in [`../research/MODULAR-PITCH-RESEARCH.md`](../research/MODULAR-PITCH-RESEARCH.md).

Corollaries:

- `NoteFormatter` trait exists day 1 even though only English (C D E F G A B) is implemented (ADR-0004); movable-do solfege slots in for Phase 4 without a refactor.
- A4 reference is a runtime parameter, not a constant, from the first commit (ADR-0005).
- `crates/` directory is created day 1 with `crates/neural-pitch-core` only. `-io` and `-ml` crates split out incrementally, when concrete need arises (YAGNI applied to crate boundaries, not to trait boundaries).

### 2.3 P2 — Pure-Rust core, Tauri only at the shell boundary

`neural-pitch-core` MUST NOT import `tauri::*`, `tauri_plugin_*`, or any frontend-shaped type. The crate compiles standalone and must remain reusable from a future CLI, mobile build, or test harness (ADR-0002).

```text
+-------------------+   +------------------+   +----------------+
| React 19 + Vite   |<->| src-tauri (shell)|<->| neural-pitch-  |
| (TS strict)       |   | (commands, IPC)  |   |   core (Rust)  |
+-------------------+   +------------------+   +----------------+
                                                    |
                                       cpal | rtrb | rustfft | ndarray
```

The shell crate (`src-tauri/`) is the only place allowed to:

- depend on `tauri`, `tauri-plugin-store`, `tauri-plugin-log`;
- own `tauri::ipc::Channel<T>` instances;
- translate `core::Error` into `Result<T, String>` for IPC.

To preserve P2 while still streaming per-frame pitch updates from the DSP worker to the UI, the core crate exposes a backend-agnostic **`FrameSink` trait**:

```rust
// crates/neural-pitch-core/src/pipeline/sink.rs
pub trait FrameSink<T>: Send {
    fn send(&self, frame: T) -> Result<(), SinkError>;
}
```

The shell crate (`src-tauri/`) is the only place that implements this trait against `tauri::ipc::Channel<T>`. The DSP worker (which itself lives in `src-tauri/`, see §6 and §9) holds a `Box<dyn FrameSink<PitchFrame>>` and never names a Tauri type.

### 2.4 P3 — The audio callback is sacred

The cpal input callback is a real-time context. The following are forbidden inside it:

| Forbidden                                                          | Reason                                          |
| ------------------------------------------------------------------ | ----------------------------------------------- |
| `Box::new`, `Vec::push` past capacity, any allocation              | Allocator may take a global lock or page-fault. |
| `std::sync::Mutex`, `RwLock`, `parking_lot::Mutex`                 | Priority inversion, unbounded wait.             |
| `println!`, `eprintln!`, `tracing::*`, `log::*`                    | I/O syscall, formatter alloc.                   |
| `tauri::*` emit, channel send other than `rtrb::Producer::push`    | Runtime crossing, alloc.                        |
| File I/O, network I/O                                              | Syscall, unbounded latency.                     |
| `unwrap`, `expect`, `panic!`, `?` on a fallible type that can fail | Unwinding in RT context.                        |

The only legal egress from the callback is `rtrb::Producer::push` of a `Copy` sample frame. Ring-buffer-full increments an `AtomicU64` drop counter and returns. A worker thread reads the counter for diagnostics. See [`../research/RESEARCH-REPORT.md`](../research/RESEARCH-REPORT.md) for the cpal/rtrb pattern survey and the cpal sole-maintainer caveat.

### 2.5 P4 — Tests before code (TDD)

Every PR introducing or changing core behavior MUST land the failing test first (or in the same commit, with the test authored before the implementation). The test tiers are detailed in §10 and locked by ADR-0016.

### 2.6 P5 — Idiomatic Rust + Linux-kernel commit hygiene

Idiom and hygiene baselines come from [`../research/REPO-CONVENTIONS-REPORT.md`](../research/REPO-CONVENTIONS-REPORT.md). Enforcement is layered (ADR-0018) and detailed in §12. Workspace-level lint policy is `unsafe_code = "forbid"`. This is a hard, absolute ban: the Rust language semantics of `forbid` mean the lint **cannot** be relaxed by an inner `#![allow(unsafe_code)]` in any member crate. If a future situation genuinely requires `unsafe` (e.g. an FFI shim), the workspace lint must be relaxed to `deny` and the relaxation justified in an ADR.

### 2.7 P6 — FOSS-first, no telemetry, local-only data

| Aspect                  | Decision                                                                             |
| ----------------------- | ------------------------------------------------------------------------------------ |
| License                 | dual `MIT OR Apache-2.0` (ADR-0001); `LICENSE-MIT` and `LICENSE-APACHE` at repo root |
| Telemetry               | none, ever, except explicit user-initiated model downloads                           |
| Crash reporting         | none                                                                                 |
| Data residence          | local only — recordings and analysis cache in platform user-data dir                 |
| Future opt-in telemetry | permitted only as opt-in with a user-visible toggle                                  |

### 2.8 P7 — Auto-detect over user-config wherever possible

The primary use case is a singer pressing Record and getting correct pitch with no setup. Any UX element that asks the user "what instrument is this?" before pitch detection runs is a defect against P7 (ADR-0007).

### 2.9 P8 — Ship visual-only Phase 1, modular for monitoring later

Phase 1 ships visual feedback only (ADR-0006). The architecture reserves a `MonitoringPipeline` slot as an additive sibling to the analysis pipeline. Adding monitoring later MUST NOT require restructuring the audio thread topology defined in P3.

### 2.10 P9 — Bake mobile-shape day 1, defer mobile builds

No mobile builds until Phase 6 (ADR-0002), but the build shape is mobile-ready from the first commit:

| Aspect     | Day 1 setting                       | Why                                                |
| ---------- | ----------------------------------- | -------------------------------------------------- |
| Lib name   | `neural_pitch_lib`                  | iOS/Android Tauri Mobile expects this shape        |
| Crate-type | `["staticlib","cdylib","rlib"]`     | Static for iOS, cdylib for Android, rlib for tests |
| Bundle ID  | `com.<org>.neuralpitch` (lowercase) | Apple/Google bundle-ID conventions                 |
| Edition    | 2024                                | —                                                  |
| MSRV       | `rust-version = "1.85"`             | Pinned to support edition 2024                     |

### 2.11 P10 — Fail loudly in tests, gracefully in production

| Layer                                                     | Error type                                            | Panic policy                                          |
| --------------------------------------------------------- | ----------------------------------------------------- | ----------------------------------------------------- |
| Library crates (`neural-pitch-core`, future `-io`, `-ml`) | `thiserror`-derived `Error` enum, one per crate       | No panics; `clippy::unwrap_used`/`expect_used` denied |
| Application layer (`src-tauri/`)                          | `anyhow::Result<T>` with `.context(...)` on bubble-up | No panics                                             |
| Tauri commands                                            | `Result<T, String>` formatted as `format!("{e:#}")`   | No panics                                             |
| Audio callback (P3)                                       | drop-counter only                                     | Forbidden absolutely                                  |
| Tests                                                     | `unwrap`/`expect` allowed and encouraged              | Loud failures preferred                               |

ADR-0015 locks this policy.

### 2.12 Interaction matrix

| Decision                                       | Principles in play                                         | Resolution                                                                |
| ---------------------------------------------- | ---------------------------------------------------------- | ------------------------------------------------------------------------- |
| No through-monitoring in Phase 1               | P8 wins over hypothetical "feature parity with tuner apps" | Visual-only ships first; monitoring is additive.                          |
| `tauri::ipc::Channel` only used in shell crate | P2 over convenience of importing Tauri in core             | Channel sender owned by shell; core emits via `FrameSink` trait.          |
| Manual instrument selector demoted             | P7 over flexibility                                        | Auto-instrument is default; selector moved to advanced settings.          |
| `crates/-io` and `crates/-ml` not split day 1  | P1 modular boundaries vs. YAGNI                            | Trait boundaries day 1; crate boundaries when there is a second consumer. |
| Audio-callback drop counter                    | P3 over visibility into drops                              | `AtomicU64` increment in callback; worker reads and logs.                 |

## 3. Recommended Stack

This section enumerates the dependency stack frozen by the 2026-06-02 design interview. Every row in the table below is authoritative and overrides any conflicting recommendation in upstream research. Sources: [`../research/RESEARCH-REPORT.md`](../research/RESEARCH-REPORT.md) §2 and [`../research/MODULAR-PITCH-RESEARCH.md`](../research/MODULAR-PITCH-RESEARCH.md) §2. Naming/edition/MSRV constraints come from [`../research/REPO-CONVENTIONS-REPORT.md`](../research/REPO-CONVENTIONS-REPORT.md).

The "Day" column uses these values:

- **D1** — present from Phase 0.
- **P1** — added during Phase 1 (live monophonic tuner).
- **P2** — added during Phase 2 (record/playback, neural backend, vocal range, vibrato).
- **P3** — added during Phase 3 (file upload, polyphonic transcription, MIDI export).
- **P4** — added during Phase 4 (ear-training games).
- **P5** — added during Phase 5 (stem separation).
- **P6** — added during Phase 6 (mobile).

Version pins below reflect the intent at the design freeze. Concrete pin values are confirmed against `crates.io` at Phase entry; the recommended-stack open questions in §15 explicitly call out version-skew items that must be reconfirmed before each phase begins. The pins below are written as major-version constraints to avoid pretending to a precision that the design freeze cannot actually deliver.

### 3.1 Stack table

| Layer                         | Choice                                            | Version target                                                 | License           | Role                                                                                                       | Day |
| ----------------------------- | ------------------------------------------------- | -------------------------------------------------------------- | ----------------- | ---------------------------------------------------------------------------------------------------------- | --- |
| Shell                         | Tauri                                             | 2.x                                                            | Apache-2.0 OR MIT | Desktop window, IPC, plugin host; mobile-shaped lib emitted day 1                                          | D1  |
| Tauri plugin: store           | `tauri-plugin-store`                              | 2.x                                                            | Apache-2.0 OR MIT | Settings JSON in OS-correct config dir                                                                     | D1  |
| Tauri plugin: log             | `tauri-plugin-log`                                | 2.x                                                            | Apache-2.0 OR MIT | OS-blessed log file paths                                                                                  | D1  |
| Tauri plugin: dialog          | `tauri-plugin-dialog`                             | 2.x                                                            | Apache-2.0 OR MIT | Native open/save dialogs (file upload Phase 3)                                                             | P3  |
| Tauri plugin: fs              | `tauri-plugin-fs`                                 | 2.x                                                            | Apache-2.0 OR MIT | Scoped filesystem reads for Phase 3 file upload (capability-gated)                                         | P3  |
| Frontend framework            | React                                             | 19.x                                                           | MIT               | UI; chosen over SvelteKit for shadcn/ui ecosystem alignment (ADR-0003)                                     | D1  |
| Frontend bundler              | Vite                                              | current LTS at design freeze                                   | MIT               | Dev server + production bundle                                                                             | D1  |
| Frontend language             | TypeScript                                        | 5.x (strict)                                                   | Apache-2.0        | `tsc --noEmit` in CI; `strict: true`                                                                       | D1  |
| State management              | Zustand                                           | 5.x                                                            | MIT               | Lightweight store; no Redux boilerplate                                                                    | D1  |
| CSS framework                 | Tailwind CSS                                      | 4.x                                                            | MIT               | Utility classes; design-token consistency                                                                  | D1  |
| Component primitives          | shadcn/ui (radix-ui)                              | radix 1.x                                                      | MIT               | Accessible primitives; copy-in pattern                                                                     | D1  |
| Frontend canvas: waveform     | wavesurfer.js                                     | 7.x                                                            | BSD-3-Clause      | Recording playback view + Spectrogram plugin                                                               | P2  |
| Frontend canvas: notation     | OpenSheetMusicDisplay                             | 1.x                                                            | BSD-3-Clause      | MusicXML rendering for transcription/ear-training                                                          | P3  |
| Audio capture + playback      | `cpal`                                            | latest 0.x at design freeze                                    | Apache-2.0 OR MIT | Cross-platform mic + speaker; sole maintainer seeking handoff (cpal issue #981) — vendor-and-patch posture | D1  |
| Decode                        | `symphonia`                                       | latest 0.x                                                     | MPL-2.0           | WAV/FLAC/MP3 day 1; AAC/M4A/OGG/AIFF behind app-level Cargo features                                       | D1  |
| Encode (recording, FLAC)      | `flacenc-rs`                                      | latest 0.x                                                     | Apache-2.0        | Pure-Rust FLAC encoder; FLAC is the default recording format (ADR-0011)                                    | P2  |
| Encode (recording, WAV)       | `hound`                                           | 3.x                                                            | Apache-2.0 OR MIT | WAV encoder behind advanced-settings opt-in                                                                | P2  |
| FFT (complex)                 | `rustfft`                                         | 6.x                                                            | Apache-2.0 OR MIT | SIMD; transitive via realfft                                                                               | D1  |
| FFT (real)                    | `realfft`                                         | 3.x                                                            | Apache-2.0 OR MIT | Real-input FFT; spectrogram + vibrato pipelines                                                            | D1  |
| Resampling                    | `rubato`                                          | 0.x (latest)                                                   | MIT               | Sinc / FFT / polynomial; needed for 16 kHz neural inputs                                                   | P2  |
| Filters                       | `biquad`                                          | 0.x                                                            | MIT               | Pre-emphasis, anti-alias, mains-hum notch                                                                  | D1  |
| Mel spectrogram               | `mel_spec`                                        | 0.x                                                            | MIT               | librosa parity; YAMNet/Basic Pitch front-end                                                               | P2  |
| SPSC ring buffer              | `rtrb`                                            | 0.x                                                            | MIT OR Apache-2.0 | Wait-free audio-callback → DSP-worker hand-off                                                             | D1  |
| MPMC channel                  | `crossbeam-channel`                               | 0.5.x                                                          | MIT OR Apache-2.0 | Fan-out from DSP worker to multiple consumers                                                              | D1  |
| Async cancellation            | `tokio-util` (`CancellationToken`)                | 0.7.x                                                          | MIT               | Cross-runtime cooperative cancellation                                                                     | D1  |
| Mutex (non-audio)             | `parking_lot`                                     | 0.12.x                                                         | MIT OR Apache-2.0 | Faster than `std::sync::Mutex`; non-poisoning                                                              | D1  |
| Tensors                       | `ndarray`                                         | major matching `ort` 2.0-rc at P2 entry (target 0.16.x)        | MIT OR Apache-2.0 | Standard input shape for `ort` / `tract`                                                                   | P2  |
| ML inference (default)        | `ort`                                             | 2.0-rc.x (`load-dynamic`)                                      | MIT OR Apache-2.0 | Wraps ONNX Runtime; PESTO + YAMNet + Basic Pitch                                                           | P2  |
| ML inference (fallback)       | `tract`                                           | 0.x (latest)                                                   | MIT OR Apache-2.0 | Pure-Rust escape hatch; mobile-friendlier                                                                  | P6  |
| Persistence (relational)      | `rusqlite`                                        | latest 0.x at design freeze (`bundled`)                        | MIT               | Recordings library + per-recording analysis cache (ADR-0012)                                               | P2  |
| Migrations                    | `refinery`                                        | 0.x                                                            | MIT OR Apache-2.0 | Versioned forward-only SQL migrations for `rusqlite`                                                       | P2  |
| Path resolution               | `directories`                                     | 5.x                                                            | MIT OR Apache-2.0 | Cross-platform user-data / config / cache dirs                                                             | D1  |
| File locking                  | `fs2`                                             | 0.4.x                                                          | MIT OR Apache-2.0 | Exclusive flock for model resolver download lock                                                           | P2  |
| HTTP client (model downloads) | `reqwest` (rustls-tls)                            | 0.12.x                                                         | MIT OR Apache-2.0 | User-initiated model downloads; rustls avoids OpenSSL on Windows/macOS                                     | P2  |
| Optional model resolver       | `hf_hub`                                          | latest 0.x                                                     | Apache-2.0        | Optional Hugging Face Hub client for model resolution where licenses permit                                | P2  |
| MIDI export                   | `midly`                                           | 0.5.x                                                          | Unlicense OR MIT  | SMF 0/1/2 with 14-bit pitch bends                                                                          | P3  |
| SoundFont synthesis           | `oxisynth`                                        | 0.x                                                            | MIT OR Apache-2.0 | SoundFont 2 player for ear-training drills (Phase 4)                                                       | P4  |
| Drum onset (Phase 5)          | `aubio-rs` or pure-Rust onset detector            | latest                                                         | dual              | Drum onset/beat tracking for percussion stem; final crate selected at Phase 5 design pass                  | P5  |
| Async runtime                 | `tokio`                                           | 1.x (`rt-multi-thread`, `macros`, `sync`, `fs`, `time`, `net`) | MIT               | Tauri commands + HTTP model downloads only; NOT used inside DSP worker                                     | D1  |
| Long-lived workers            | `std::thread`                                     | std                                                            | n/a               | DSP worker + offline analysis jobs                                                                         | D1  |
| UI streaming                  | `tauri::ipc::Channel<T>`                          | Tauri 2.x                                                      | Apache-2.0 OR MIT | DSP-to-UI per-frame stream; shell-side only                                                                | D1  |
| TS-side type generation       | `ts-rs` or `specta` (final pick at Phase 1 entry) | latest                                                         | MIT               | Generates TS types for Tauri command payloads                                                              | D1  |
| Errors (libs)                 | `thiserror`                                       | 1.x                                                            | MIT OR Apache-2.0 | One enum per crate at the library boundary                                                                 | D1  |
| Errors (app)                  | `anyhow`                                          | 1.x                                                            | MIT OR Apache-2.0 | Tauri commands; `.context(...)` on bubble-up                                                               | D1  |
| Logging                       | `tracing` + `tracing-subscriber`                  | 0.1.x / 0.3.x                                                  | MIT               | Structured spans/events; pretty-stderr dev, JSON-rotating-file release                                     | D1  |
| Serialization                 | `serde` + `serde_json`                            | 1.x                                                            | MIT OR Apache-2.0 | Settings, IPC payloads, model manifest                                                                     | D1  |
| Config (TOML)                 | `toml`                                            | 0.8.x                                                          | MIT OR Apache-2.0 | `models.toml` manifest parsing                                                                             | P2  |
| UUIDs                         | `uuid`                                            | 1.x (`v7`, `serde`)                                            | Apache-2.0 OR MIT | Stable recording IDs (UUIDv7 for time-ordered keys)                                                        | P2  |
| Time                          | `chrono`                                          | 0.4.x (`serde`)                                                | MIT OR Apache-2.0 | Recording timestamps, filename formatting                                                                  | P2  |
| Property testing              | `proptest`                                        | 1.x (dev-dep)                                                  | MIT OR Apache-2.0 | Round-trip + windowing invariants                                                                          | D1  |
| Test harness                  | `cargo test`                                      | n/a                                                            | n/a               | Tier 1 synthesized signals + golden tables                                                                 | D1  |
| Benchmarks                    | `criterion`                                       | 0.5.x (dev-dep)                                                | Apache-2.0 OR MIT | Pitch-detector and FFT micro-bench; not on CI hot path                                                     | P1  |
| Build profile lint            | `cargo deny`                                      | 0.x                                                            | Apache-2.0 OR MIT | License + advisory + duplicate-version gate                                                                | D1  |
| Lint                          | `clippy`                                          | toolchain-pinned                                               | dual              | `-D warnings`; `pedantic` warn; `unwrap_used`/`expect_used`/`panic` deny                                   | D1  |
| Format                        | `rustfmt`                                         | toolchain-pinned                                               | dual              | `cargo fmt --check` in pre-commit + CI                                                                     | D1  |
| Toolchain pin                 | `rust-toolchain.toml`                             | edition 2024, MSRV 1.85                                        | n/a               | `rustfmt` + `clippy` components declared                                                                   | D1  |
| Pre-commit framework          | `pre-commit`                                      | 3.x                                                            | MIT               | Hooks driver                                                                                               | D1  |
| Frontend lint                 | ESLint + Prettier                                 | 9.x / 3.x                                                      | MIT               | `eslint --max-warnings 0`; ts-aware config                                                                 | D1  |
| Build/release                 | GitHub Actions                                    | n/a                                                            | n/a               | Linux/macOS/Windows × stable/beta matrix                                                                   | D1  |
| Telemetry / crash reporting   | (none)                                            | n/a                                                            | n/a               | Local-only; no network calls except user-initiated model downloads                                         | D1  |

### 3.2 Day-1 `Cargo.toml` workspace dependencies

The root `Cargo.toml` declares only the D1 subset. Phase-gated additions (`ort`, `rubato`, `mel_spec`, `ndarray`, `rusqlite`, `refinery`, `flacenc-rs`, `midly`, `criterion`, `oxisynth`, `reqwest`, `hf_hub`, `fs2`, …) land in subsequent phases when the first consumer crate needs them.

```toml
# Cargo.toml — root workspace (Day 1 / Phase 0)
[workspace]
resolver = "3"
members  = ["src-tauri", "crates/neural-pitch-core"]

[workspace.package]
edition       = "2024"
rust-version  = "1.85"
license       = "MIT OR Apache-2.0"
repository    = "https://github.com/<org>/neural-pitch"
version       = "0.1.0"

[workspace.lints.rust]
unsafe_code = "forbid"
missing_docs = "warn"

[workspace.lints.clippy]
pedantic     = { level = "warn", priority = -1 }
unwrap_used  = "deny"
expect_used  = "deny"
panic        = "deny"
todo         = "warn"
```

The workspace resolver is locked at `"3"` — the Edition-2024 default. Any embedded TOML in this document or in the repository must match.

```toml
# crates/neural-pitch-core/Cargo.toml — pure-Rust core, NO Tauri imports
[package]
name        = "neural-pitch-core"
version.workspace      = true
edition.workspace      = true
rust-version.workspace = true
license.workspace      = true
repository.workspace   = true

[lints]
workspace = true

[dependencies]
rustfft           = { workspace = true }
realfft           = { workspace = true }
biquad            = { workspace = true }
rtrb              = { workspace = true }
crossbeam-channel = { workspace = true }
parking_lot       = { workspace = true }
thiserror         = { workspace = true }
tracing           = { workspace = true }
serde             = { workspace = true }
directories       = { workspace = true }

[dev-dependencies]
proptest = { workspace = true }

[features]
default = ["decoder-symphonia"]
decoder-symphonia = ["dep:symphonia"]
neural  = []   # Phase 2: gates ort, ndarray, mel_spec when added
pyin    = []   # Phase 2: gates pyin backend
dataset = []   # Phase 2: gates Tier-3 dataset-fetch tests
debug-overlay = []   # Phase 1+: enables DebugFrame emission for the dev overlay
```

```toml
# src-tauri/Cargo.toml — application shell
[package]
name        = "neural-pitch"
version.workspace      = true
edition.workspace      = true
rust-version.workspace = true
license.workspace      = true

[lib]
name       = "neural_pitch_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[lints]
workspace = true

[build-dependencies]
tauri-build = { workspace = true }

[dependencies]
neural-pitch-core  = { path = "../crates/neural-pitch-core" }
tauri              = { workspace = true }
tauri-plugin-store = { workspace = true }
tauri-plugin-log   = { workspace = true }
cpal               = { workspace = true }
symphonia          = { workspace = true }
tokio              = { workspace = true }
tokio-util         = { workspace = true }
anyhow             = { workspace = true }
thiserror          = { workspace = true }
tracing            = { workspace = true }
tracing-subscriber = { workspace = true }
serde              = { workspace = true }
serde_json         = { workspace = true }
directories        = { workspace = true }
```

### 3.3 Notes on specific pins

| Decision                                | Rationale                                                                                                                                                                                                                                                                                            |
| --------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cpal` pinned to a specific `0.x` patch | Sole maintainer is publicly seeking handoff (cpal issue #981); pin reduces accidental surface-area churn. The exact patch is fixed at design freeze and recorded in the lockfile.                                                                                                                    |
| `symphonia` default features only       | Day-1 build needs WAV/FLAC/MP3 only; AAC/M4A/OGG/AIFF land as app-level Cargo features.                                                                                                                                                                                                              |
| FLAC encoder = `flacenc-rs`             | Pure-Rust, actively maintained, FLAC-only. Symphonia's encoder support is read-focused; `flacenc-rs` is the encoder we actually ship.                                                                                                                                                                |
| `ort = "2.0-rc.x"` (when added P2)      | Release-candidate is the actively maintained line. `load-dynamic` keeps the ONNX Runtime DLL/.so/.dylib out of the binary.                                                                                                                                                                           |
| `tract` deferred to P6                  | Cited only as the mobile/pure-Rust fallback.                                                                                                                                                                                                                                                         |
| `rusqlite` over `sqlx`                  | Synchronous API is correct for this workload (analysis cache writes off the audio path); avoids dragging tokio into the storage layer. `bundled` ships SQLite to dodge per-platform system-library version skew.                                                                                     |
| `refinery` for migrations               | Versioned forward-only SQL files (`V0001__init.sql` etc.) checked into the repo and embedded in the binary.                                                                                                                                                                                          |
| `directories` for user-data paths       | Single cross-platform crate; avoids hand-rolling per-OS path logic.                                                                                                                                                                                                                                  |
| `fs2` for file locks                    | Exclusive flock during model download. Note the platform difference: Windows releases the lock on file-handle close (so process crash auto-releases); on Linux/macOS the lock is also handle-scoped via `fcntl(F_OFD_*)`. A janitor sweep on app startup removes any stale `<target>.partial` files. |
| `reqwest` with `rustls-tls`             | rustls avoids OpenSSL on Windows/macOS — significantly easier cross-compilation, especially for Phase 6 mobile.                                                                                                                                                                                      |
| Edition 2024 + MSRV 1.85                | Per [`../research/REPO-CONVENTIONS-REPORT.md`](../research/REPO-CONVENTIONS-REPORT.md); 1.85 is the stable that ships Edition 2024 support.                                                                                                                                                          |
| Dual-license `MIT OR Apache-2.0`        | Per Rust ecosystem convention (ADR-0001); both `LICENSE-MIT` and `LICENSE-APACHE` files at repo root.                                                                                                                                                                                                |

## 4. Repository and Workspace Layout

This section specifies the on-disk shape of the `neural-pitch` repository on day 0: every file and directory committed (or deliberately gitignored) at the moment Phase 0 lands. The shape is locked verbatim against the recommendation in [`../research/REPO-CONVENTIONS-REPORT.md`](../research/REPO-CONVENTIONS-REPORT.md), tempered by YAGNI on crate boundaries.

The layout supports four invariants drawn from the locked decisions:

1. Top-level Cargo workspace (`resolver = "3"`) with `src-tauri/` as a workspace member and additional library crates living under `crates/`.
2. Web frontend lives at the repo root (Tauri 2.x canonical), not nested.
3. Mobile-ready `src-tauri/` shape from day 1: `[lib] name = "neural_pitch_lib"`, `crate-type = ["staticlib","cdylib","rlib"]`, `main.rs` is a one-line shim, `lib.rs` holds all entry logic.
4. ML weights, build outputs, and dataset slices are gitignored; their fetch is scripted and documented.

### 4.1 Day-0 directory tree

```
neural-pitch/
├─ Cargo.toml                       # workspace root manifest; [workspace] members, resolver="3"
├─ Cargo.lock                       # tracked (binary/app workspace, not a published library)
├─ rust-toolchain.toml              # pins toolchain channel + components (rustfmt, clippy)
├─ rustfmt.toml                     # formatting policy (edition 2024, max_width, etc.)
├─ deny.toml                        # cargo-deny config: licenses, advisories, bans, sources
├─ .editorconfig                    # cross-editor whitespace/EOL/charset normalization
├─ .gitignore                       # Rust + Node + Tauri + models/ + tests/data/ + dist/ + target/
├─ .pre-commit-config.yaml          # pre-commit framework manifest (fmt, clippy, lint, commit-msg)
├─ README.md                        # project overview, build instructions, status badge, license note
├─ LICENSE-MIT                      # MIT license text
├─ LICENSE-APACHE                   # Apache-2.0 license text
├─ CHANGELOG.md                     # Keep-a-Changelog format; "Unreleased" section seeded
├─ CONTRIBUTING.md                  # operational rulebook (commit format, DCO, validation gate)
├─ models.toml                      # ML model manifest (empty placeholder day 0; populated Phase 2+)
│
├─ index.html                       # Vite entry HTML
├─ package.json                     # frontend deps + scripts (vite, tsc, eslint, prettier)
├─ package-lock.json                # tracked lockfile for reproducible frontend installs
├─ vite.config.ts                   # Vite config: tauri plugin, alias, build target, dev server
├─ tsconfig.json                    # TS strict mode
├─ tsconfig.node.json               # narrow tsconfig for vite.config.ts itself
├─ src/                             # frontend source (React 19 + Zustand + Tailwind + shadcn/ui)
│  ├─ main.tsx
│  ├─ App.tsx
│  ├─ ipc/                          # Tauri command wrappers (typed)
│  ├─ store/                        # Zustand stores
│  ├─ components/                   # UI components (shadcn primitives composed here)
│  ├─ styles/                       # Tailwind globals + tokens
│  └─ i18n/                         # centralized strings
├─ public/                          # static assets served verbatim by Vite
│  └─ favicon.svg
├─ dist/                            # GITIGNORED — Vite build output
│
├─ src-tauri/                       # Tauri Rust app (workspace member)
│  ├─ Cargo.toml
│  ├─ build.rs                      # tauri-build invocation
│  ├─ tauri.conf.json               # Tauri config: bundle id com.<org>.neuralpitch, plugins
│  ├─ src/
│  │  ├─ lib.rs                     # ALL logic; #[cfg_attr(mobile, tauri::mobile_entry_point)] entry
│  │  └─ main.rs                    # one-line shim: fn main() { neural_pitch_lib::run() }
│  ├─ capabilities/
│  │  └─ default.json               # capability set for default window
│  ├─ icons/                        # platform icons (png, ico, icns)
│  ├─ gen/                          # GITIGNORED — mobile-generated artifacts (deferred Phase 6)
│  └─ target/                       # GITIGNORED — Cargo build output for src-tauri member
│
├─ crates/                          # workspace member crates (incremental — YAGNI)
│  └─ neural-pitch-core/            # DSP + pitch detection trait + analyzers (day-0 ONLY)
│
├─ tests/                           # workspace-level integration tests + fixtures
│  ├─ fixtures/                     # COMMITTED small audio fixtures
│  │  └─ .gitkeep
│  └─ data/                         # GITIGNORED — large dataset slices
│
├─ models/                          # GITIGNORED — runtime ML weights
│
├─ docs/
│  ├─ research/                     # research reports cited throughout this design
│  │  ├─ RESEARCH-REPORT.md
│  │  ├─ MODULAR-PITCH-RESEARCH.md
│  │  └─ REPO-CONVENTIONS-REPORT.md
│  ├─ design/                       # design docs (this document is docs/design/DESIGN.md)
│  │  └─ DESIGN.md
│  ├─ adr/                          # Architecture Decision Records (numbered ADR-NNNN)
│  │  ├─ README.md
│  │  └─ 0001-...md ... 0018-...md
│  └─ licenses/                     # third-party + model license register
│     └─ REGISTER.md
│
├─ scripts/
│  ├─ install-hooks.sh              # bootstraps pre-commit hooks
│  ├─ fetch-models.sh               # Phase 2+ — resolves models.toml entries to ./models/
│  └─ fetch-test-data.sh            # Phase 2+ — populates gitignored tests/data/
│
├─ .cargo/
│  └─ config.toml                   # rustflags = ["-D","warnings"]
│
├─ .github/
│  ├─ workflows/
│  │  ├─ ci.yml                     # commit-lint, fmt, clippy -D warnings, typecheck, test matrix, deny
│  │  └─ release.yml                # tag-driven Tauri bundle release
│  ├─ ISSUE_TEMPLATE/
│  └─ PULL_REQUEST_TEMPLATE.md
│
├─ node_modules/                    # GITIGNORED
└─ target/                          # GITIGNORED — Cargo workspace build output
```

This canonical design document lives at `docs/design/DESIGN.md`. All `../research/...` references throughout this file resolve relative to that path.

### 4.2 YAGNI posture on crate splits

[`../research/REPO-CONVENTIONS-REPORT.md`](../research/REPO-CONVENTIONS-REPORT.md) explicitly warns that pre-created crate boundaries are a refactoring tax with no offsetting benefit until a concrete consumer appears. The locked decision is to _plan_ the full fan-out but only _create_ the boundaries on demand.

| Crate                      | Day 0    | Trigger to split                                                                                                                                                                                                                                                                                       |
| -------------------------- | -------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `crates/neural-pitch-core` | Created  | Always present — DSP, traits, analyzers, note model.                                                                                                                                                                                                                                                   |
| `crates/neural-pitch-io`   | Deferred | When a second audio backend (oboe/coreaudio) or a non-Tauri consumer (CLI) demands an I/O abstraction crate not naturally housed in `core`. The DSP worker and `tauri::ipc::Channel`-backed `FrameSink` impl currently live in `src-tauri/`; if a CLI consumer needs them, they migrate to this crate. |
| `crates/neural-pitch-ml`   | Deferred | When a second inference backend (e.g., tract fallback alongside ort, or burn/candle PESTO from Phase 7) makes the ML surface large enough to warrant isolation.                                                                                                                                        |

Until those triggers fire, ML and I/O modules live as sub-modules inside `neural-pitch-core` (or, for Tauri-coupled glue, inside `src-tauri/src/lib.rs`).

## 5. neural-pitch-core Crate — Module Layout and Public API

This section specifies the module tree of the `neural-pitch-core` crate, the canonical `PitchEstimator` trait and supporting types, and the rules a contributor must follow to add a new pitch-detection backend (ADR-0007). The crate is the only library crate that exists day 1; `neural-pitch-io` and `neural-pitch-ml` are deferred until a concrete reuse pressure justifies the split. The crate is pure-Rust, has no Tauri imports (P2), and is the unit of reuse for future CLI, mobile, and embedded targets.

### 5.1 Crate identity

| Field                  | Value                                        |
| ---------------------- | -------------------------------------------- |
| Cargo name             | `neural-pitch-core`                          |
| Path in workspace      | `crates/neural-pitch-core/`                  |
| Edition                | `2024`                                       |
| `rust-version`         | `1.85`                                       |
| License                | `MIT OR Apache-2.0`                          |
| Crate type             | `rlib` (default)                             |
| External Tauri symbols | none (pure-Rust, mobile-portable)            |
| Default features       | `decoder-symphonia`                          |
| Optional features      | `neural`, `pyin`, `dataset`, `debug-overlay` |

The mobile-shaped multi-crate-type (`staticlib`, `cdylib`, `rlib`) lives in `src-tauri/Cargo.toml` (P9); this core crate is plain `rlib` so it can be reused by a future CLI, by tests, or by a Phase-6 mobile shell that wraps it.

### 5.2 `src/` module tree

```text
crates/neural-pitch-core/
├─ Cargo.toml
├─ src/
│  ├─ lib.rs                    # crate root: re-exports + #![forbid(unsafe_code)]
│  ├─ prelude.rs                # convenience re-exports for downstream callers
│  ├─ error.rs                  # crate-wide thiserror enum (NeuralPitchError)
│  ├─ settings.rs               # Settings, schema_version, serde defaults, migrations
│  ├─ smoothing.rs              # ContourSmoother
│  ├─ voicing.rs                # VoiceActivityGate
│  │
│  ├─ audio/
│  │   ├─ mod.rs                # AudioFrame, AudioBlock
│  │   ├─ decoder.rs            # AudioDecoder trait + SymphoniaDecoder (cfg-feature)
│  │   └─ encoder.rs            # AudioEncoder trait, FlacEncoder, WavEncoder
│  │
│  ├─ music/
│  │   ├─ mod.rs                # re-exports
│  │   ├─ note.rs               # frequency_to_note, midi_to_hz, cents math, profiles
│  │   └─ format.rs             # NoteFormatter trait, EnglishFormatter
│  │
│  ├─ pitch/
│  │   ├─ mod.rs                # PitchEstimator trait, F0Frame, EstimatorConfig
│  │   ├─ factory.rs            # Backend enum, make_estimator()
│  │   ├─ auto_prior.rs         # running median + power-weighted F0 histogram (Phase 1);
│  │   │                        # YAMNet wrapper (Phase 2, cfg-feature = "neural")
│  │   ├─ yin.rs                # YinMpmEstimator — Phase 1, ships day 1
│  │   ├─ pyin.rs               # PYinEstimator — Phase 2, cfg(feature = "pyin")
│  │   └─ neural/               # Phase 2.2, all under cfg(feature = "neural") — ADR-0020
│  │       ├─ mod.rs            # feature-gated re-exports; absent surface when feature off
│  │       ├─ pesto.rs          # PestoEstimator — default neural backend; ONNX via `ort` + `ndarray`;
│  │       │                    # weights resolved at runtime by models.toml resolver, NOT bundled
│  │       ├─ crepe_tiny.rs     # CrepeTinyEstimator — MIT fallback; same runtime-asset contract
│  │       └─ viterbi.rs        # shared Viterbi DP decoder for neural posteriors (PESTO + CREPE-tiny)
│  │
│  └─ pipeline/
│      ├─ mod.rs                # re-exports
│      ├─ sink.rs               # FrameSink<T> trait — backend-agnostic sink
│      ├─ live_tuner.rs         # LiveTunerPipeline — Phase 1
│      ├─ recorder.rs           # RecorderPipeline — Phase 2
│      └─ song_analysis.rs      # SongAnalysisPipeline — Phase 5
│
├─ benches/                     # criterion benches (Phase 1: YIN cost; Phase 2: PESTO)
└─ tests/
   ├─ fixtures/                 # Philharmonia voice fixtures (Tier 2, in-repo)
   └─ data/                     # gitignored, fetched by scripts/fetch-test-data.sh
```

The DSP worker itself does **not** live in `neural-pitch-core` — it lives in `src-tauri/` (or a future `crates/neural-pitch-io` crate) precisely because the worker holds a `Box<dyn FrameSink<PitchFrame>>` that is implemented against `tauri::ipc::Channel<T>` in the shell. Core defines the abstract `FrameSink` trait; shell implements it. This is how P2 is preserved.

### 5.3 The `PitchEstimator` trait

```rust
//! crates/neural-pitch-core/src/pitch/mod.rs

use std::path::PathBuf;

#[derive(Clone, Copy, Debug)]
pub struct F0Frame {
    /// Estimated fundamental in Hertz. Always > 0 when `voiced` is true.
    pub f0_hz: f32,
    /// Estimator-reported confidence, normalised to [0.0, 1.0].
    pub confidence: f32,
    /// True when the estimator's voicing decision agrees with any caller-side gate.
    pub voiced: bool,
    /// Sample-accurate timestamp; reset by `reset()`.
    pub timestamp_samples: u64,
}

#[derive(Clone, Debug)]
pub struct EstimatorConfig {
    pub sample_rate_hz: u32,
    pub window_size: usize,
    pub hop_size: usize,
    pub fmin_hz: f32,
    pub fmax_hz: f32,
    pub instrument_hint: Option<InstrumentHint>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstrumentHint {
    Voice, Guitar, Bass, Piano, Violin, Generic,
}

#[derive(thiserror::Error, Debug)]
pub enum EstimatorError {
    #[error("model file not found: {0}")]
    ModelNotFound(PathBuf),
    #[error("ort runtime error: {0}")]
    Ort(String),
    #[error("input frame size {got} != expected {want}")]
    WindowMismatch { got: usize, want: usize },
    #[error("backend disabled at compile time: feature = \"{0}\"")]
    FeatureDisabled(&'static str),
    #[error("invalid configuration: {0}")]
    InvalidConfig(String),
}

/// Estimator instances are not designed to be shared across threads; pipelines own
/// them exclusively. The `Send` bound is required because pipelines hand estimators
/// to dedicated DSP workers; no `Sync` bound is required and impls SHOULD NOT
/// introduce internal `Mutex`es.
pub trait PitchEstimator: Send {
    fn name(&self) -> &'static str;
    fn config(&self) -> &EstimatorConfig;
    fn process(&mut self, samples: &[f32]) -> Result<Option<F0Frame>, EstimatorError>;
    fn reset(&mut self);
}
```

### 5.4 The `Backend` enum and `make_estimator` constructor

```rust
//! crates/neural-pitch-core/src/pitch/factory.rs

use std::path::Path;
use crate::pitch::{Backend, EstimatorConfig, EstimatorError, PitchEstimator};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Backend {
    YinMpm,
    PYin,
    OnnxPesto,
    OnnxCrepeTiny,
}

/// Construct a boxed estimator for the requested backend.
///
/// `model_root` is the resolved directory containing ONNX weights. `None` is the
/// correct value for classical backends (YIN/MPM, pYIN). Neural backends require
/// `Some(...)` and return `EstimatorError::ModelNotFound` if the path is missing
/// the requested weights.
pub fn make_estimator(
    backend: Backend,
    cfg: EstimatorConfig,
    model_root: Option<&Path>,
) -> Result<Box<dyn PitchEstimator>, EstimatorError> {
    match backend {
        Backend::YinMpm => Ok(Box::new(crate::pitch::yin::YinMpmEstimator::new(cfg)?)),

        #[cfg(feature = "pyin")]
        Backend::PYin => Ok(Box::new(crate::pitch::pyin::PYinEstimator::new(cfg)?)),
        #[cfg(not(feature = "pyin"))]
        Backend::PYin => Err(EstimatorError::FeatureDisabled("pyin")),

        #[cfg(feature = "neural")]
        Backend::OnnxPesto => {
            let root = model_root.ok_or_else(|| {
                EstimatorError::InvalidConfig("OnnxPesto requires model_root".into())
            })?;
            Ok(Box::new(crate::pitch::neural::pesto::PestoEstimator::new(cfg, root)?))
        }
        #[cfg(not(feature = "neural"))]
        Backend::OnnxPesto => Err(EstimatorError::FeatureDisabled("neural")),

        #[cfg(feature = "neural")]
        Backend::OnnxCrepeTiny => {
            let root = model_root.ok_or_else(|| {
                EstimatorError::InvalidConfig("OnnxCrepeTiny requires model_root".into())
            })?;
            Ok(Box::new(crate::pitch::neural::crepe_tiny::CrepeTinyEstimator::new(cfg, root)?))
        }
        #[cfg(not(feature = "neural"))]
        Backend::OnnxCrepeTiny => Err(EstimatorError::FeatureDisabled("neural")),
    }
}
```

The `Option<&Path>` shape replaces an earlier "always required, ignored for classical" signature so that Phase-1 callers never need to invent a sentinel path.

### 5.5 Cargo feature matrix

| Feature             | Default | Pulls in                   | Effect                                                                                                      |
| ------------------- | ------- | -------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `decoder-symphonia` | on      | `symphonia` (wav/flac/mp3) | Enables `audio::decoder::SymphoniaDecoder`.                                                                 |
| `pyin`              | off     | `pyin` (Sytronik port)     | Enables `Backend::PYin`. Phase 2.                                                                           |
| `neural`            | off     | `ort 2.0-rc`, `ndarray`    | Enables `Backend::OnnxPesto`, `Backend::OnnxCrepeTiny`, neural Viterbi decoder, YAMNet auto-prior. Phase 2. |
| `dataset`           | off     | none (test-only)           | Enables Tier 3 dataset-slice tests. Phase 2.                                                                |
| `debug-overlay`     | off     | none                       | Enables `DebugFrame` emission and the `toggle_debug_overlay` command for the dev overlay. Phase 1+.         |

### 5.6 Crate-level invariants

1. No `unsafe` (workspace lint `unsafe_code = "forbid"`).
2. No `unwrap`, `expect`, or `panic!` outside `#[cfg(test)]`.
3. No allocation on the hot path of `process` after the first call.
4. No file or network I/O from `process`. Model loading happens in `new()`.
5. No logging, no `tracing` events, no `println!` from inside the cpal callback path. Worker-thread emission only, via atomic counters that the worker reads.
6. All public functions take `a4_hz` as a parameter — never as module-level state (ADR-0005).
7. All public types implement `Debug`. `F0Frame` and `EstimatorConfig` are `Copy` and `Clone` respectively.

## 6. Real-Time Audio Pipeline

The realtime path is the latency-critical spine of NeuralPitch Phase 1. It captures microphone samples via `cpal`, hands them off without locks across thread boundaries, runs YIN/MPM in a worker thread, and streams `PitchUpdate` frames to the WebView for canvas rendering.

### 6.1 Thread topology

```text
+----------------------+        rtrb SPSC ring         +---------------------+
| cpal input callback  | ----- f32 samples (Copy) ---> |  DSP worker thread  |
| (RT context)         |                               |  (std::thread)      |
| - no alloc / lock /  |                               | - YIN/MPM           |
|   syscall / panic    |  AtomicU64 drop-counter (RT)  | - smoother          |
|                      |  ----------------------------> | - VAD               |
+----------------------+                               | - emits PitchFrame  |
                                                       +----------+----------+
                                                                  |
                                                  Box<dyn FrameSink<PitchFrame>>
                                                                  |
                                          +-----------------------v-----------------------+
                                          | shell-side TauriChannelSink                   |
                                          | (src-tauri/, holds tauri::ipc::Channel<T>)    |
                                          +-----------------------+-----------------------+
                                                                  |
                                                                  v
                                                       WebView (React canvas, rAF)
```

The worker lives in `src-tauri/` (or, on Phase-6 mobile, in a future `crates/neural-pitch-io` that may depend on a mobile-friendly transport). It holds a `Box<dyn FrameSink<PitchFrame>>` from `neural-pitch-core`; the sink implementation lives in `src-tauri/` and wraps a `tauri::ipc::Channel<PitchFrame>`. This indirection is what preserves P2 — `neural-pitch-core` itself never names a Tauri type.

### 6.2 The audio callback rules

P3 (§2.4) is the canonical statement. Restated for this section:

- The cpal input callback is a real-time context.
- Only `rtrb::Producer::push` of a `Copy` sample frame is permitted as egress.
- `slots()`-or-equivalent capacity check is performed; on full ring, an `AtomicU64` drop counter is incremented and the sample is discarded.
- No `unwrap`, `expect`, `panic!`, allocation, lock, syscall, or formatter call appears anywhere reachable from the callback.

### 6.3 Latency budget

| Stage                              | Typical                    | Worst-case                   |
| ---------------------------------- | -------------------------- | ---------------------------- |
| OS audio capture buffer            | 5–10 ms                    | 20 ms                        |
| Ring-buffer hand-off               | < 0.1 ms                   | < 0.1 ms                     |
| DSP analysis (YIN @ 2048 / 48 kHz) | 5–10 ms                    | 20 ms                        |
| Smoothing + VAD                    | < 1 ms                     | 2 ms                         |
| FrameSink → IPC → WebView          | 1–3 ms (JSON Channel)      | 5–10 ms (Windows worst-case) |
| Canvas render via rAF              | 16 ms (one frame at 60 Hz) | 33 ms (two frames at p99)    |
| **Mic-to-screen total**            | **~30–45 ms**              | **~60–70 ms**                |

Phase 1 acceptance (§13) is stated as p50 ≤ 45 ms and p99 ≤ 70 ms, measured mic-to-screen on the test rig described in the Tier-1 hardware-sanity check in §10.

### 6.4 Buffer sizing

The single rule: ring-buffer capacity is **3 × the active analyzer's `EstimatorConfig::window_size`**, rounded up to the next power of two.

| Active estimator profile                   | `window_size` | Ring capacity (samples) |
| ------------------------------------------ | ------------- | ----------------------- |
| Live tuner — voice (default, YIN @ 48 kHz) | 2048          | 8192                    |
| Live tuner — bass profile (YIN @ 48 kHz)   | 4096          | 16384                   |
| PESTO offline (Phase 2)                    | 960           | 4096                    |

Rationale: capacity scales with the analyzer profile, not a hardcoded duration. PESTO is offline-only in Phase 2 — the live tuner stays YIN even when `feature = "neural"` is enabled. Phase-2-and-beyond live PESTO would require its own row in this table; that is deferred to whatever phase actually moves PESTO to the live path.

A reference consumer-side idiom — using `expect` on a guarded read, with the `expect_used` lint locally relaxed for this single call site — looks like:

```rust
// Inside the DSP worker, NOT the audio callback.
if cons.slots() >= window_size {
    #[allow(clippy::expect_used,
        reason = "slots() guard above guarantees window_size samples are available")]
    let chunk = cons.read_chunk(window_size)
        .expect("slots() guard guarantees window_size available");
    // ... process(chunk) ...
}
```

### 6.5 Sample-rate negotiation

cpal exposes the device's supported configs. The selection policy is:

1. Prefer 48 000 Hz (PESTO native; OS default on macOS and Windows).
2. Fall back to 44 100 Hz if 48 kHz is not offered.
3. If neither is available, use the device's default sample rate; the DSP worker resamples to 48 kHz via `rubato` (Phase 2+) only when neural backends are active.

For Phase 1 (YIN only), the analyzer adapts its `fmin_hz`/`fmax_hz` to the actual sample rate; no resample is required.

### 6.6 Cancellation contract

Long-running offline analyses (Phase 2+) honour a `tokio_util::CancellationToken` passed in by the Tauri command. The pattern is:

```rust
async fn analyze_recording(
    handle: AppHandle,
    recording_id: Uuid,
    cancel: CancellationToken,
) -> Result<AnalysisSummary, String> {
    let mut estimator = make_estimator(
        Backend::PYin,
        EstimatorConfig::default_for_voice(),
        None,
    ).map_err(|e| format!("{e:#}"))?;

    for window in windows {
        if cancel.is_cancelled() {
            return Err("cancelled".into());
        }
        let _ = estimator.process(&window).map_err(|e| format!("{e:#}"))?;
    }
    Ok(summarise())
}
```

Each long-running command constructs and owns its own estimator instance; the live-tuner DSP-worker estimator is separate and never crosses into command code.

### 6.7 Reserved seat for `MonitoringPipeline`

P8 reserves a seat for an additive `MonitoringPipeline` sibling. The shape is sketched here so that the audio thread topology defined above remains stable even when monitoring is added later: a second cpal output stream is opened, fed by an additional rtrb ring written by either the input callback (zero-latency loopback) or the DSP worker (post-processing path). The analysis pipeline does not change.

## 7. Frontend Architecture

### 7.1 Stack summary

The frontend stack is locked at React 19 + Vite + TypeScript strict + Zustand + Tailwind + shadcn/ui (ADR-0003). Vite version is fixed at the current LTS at design freeze; the recommended-stack open questions in §15 call out that Vite 5/6/7 selection must be reconfirmed at Phase-0 close.

### 7.2 Module layout

```
src/
├─ main.tsx                 # React 19 root mount
├─ App.tsx                  # top-level component, route shell
├─ ipc/                     # typed Tauri command wrappers; one TS file per Tauri command
├─ store/                   # Zustand stores (settings, tuner state, recordings list)
├─ components/              # composed UI; shadcn primitives are copied into ui/
│  ├─ ui/                   # shadcn-copied primitives
│  ├─ tuner/                # live tuner page
│  ├─ recordings/           # Phase 2+
│  └─ ear-training/         # Phase 4+
├─ styles/                  # Tailwind globals + tokens
└─ i18n/                    # centralized strings (English-only day 0)
```

### 7.3 Real-time pitch path

The live-tuner canvas bypasses React reactivity. The pattern:

1. A `usePitchStream` hook owns a `tauri::ipc::Channel<PitchUpdate>` instance.
2. The hook calls `start_pitch_stream({ onFrame: channel })` when capture is started (Settings → device select), **not** at app mount.
3. Incoming frames are written into a `useRef<RingBuffer>` — never into Zustand or React state.
4. A `requestAnimationFrame` loop reads the ring and draws on `<canvas>` directly.
5. On navigation away from the Tuner page, the hook calls `stop_pitch_stream` and tears down the Channel, paying a re-handshake cost on return rather than serialising frames into a hidden tab.

`PitchUpdate` carries a small fixed payload (`f0_hz: f32`, `cents: f32`, `confidence: f32`, `voiced: bool`, `timestamp_samples: u64`) and is transmitted as JSON via `tauri::ipc::Channel<PitchUpdate>`. The per-frame JSON cost at ~93 Hz is negligible.

`DebugFrame` (see §11) carries a `Vec<f32>` difference function of size up to 2048; that payload is transmitted via the binary `InvokeResponseBody::Raw` mode rather than JSON to avoid 8 KB+ of JSON per frame.

### 7.4 Tauri command wrappers

TS-side type definitions are generated from Rust types via `ts-rs` or `specta` (final pick at Phase 1 entry; recorded as an open question in §15). The `src/ipc/` directory contains one wrapper per Tauri command, each invoking through the typed `invoke<T>` helper.

### 7.5 Phase additions

| Phase | Frontend addition                                                                                                                                                                        | Notes                                                                                 |
| ----- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------- |
| 1     | Tuner page, settings drawer, device picker                                                                                                                                               | English-only strings; movable-do solfege deferred.                                    |
| 2     | Recordings list, playback waveform via wavesurfer.js, vocal-range chart                                                                                                                  | Settings expanded with neural-backend toggle (gated on `feature = "neural"` in core). |
| 3     | File picker (uses `tauri-plugin-dialog` + `tauri-plugin-fs`; capability JSON updated to grant scoped read on user-picked paths), MIDI export action, OpenSheetMusicDisplay notation view |                                                                                       |
| 4     | Ear-training page (movable-do solfege drills), karaoke pitch ribbon                                                                                                                      | SoundFont synthesis driven from Tauri via `oxisynth`.                                 |
| 5     | Per-stem analysis view                                                                                                                                                                   |                                                                                       |
| 6     | Mobile-shaped layouts (Tauri Mobile)                                                                                                                                                     |                                                                                       |

## 8. Persistence and Storage

### 8.1 Storage layout

| Layer                               | Format                        | Backend                            | Purpose                                             |
| ----------------------------------- | ----------------------------- | ---------------------------------- | --------------------------------------------------- |
| Settings                            | JSON                          | `tauri-plugin-store` (ADR-0013)    | User preferences                                    |
| Recordings library + analysis cache | SQLite                        | `rusqlite` + `refinery` (ADR-0012) | Recording metadata + per-recording analyzer outputs |
| Audio files                         | FLAC (default) / WAV (opt-in) | filesystem                         | Single-take voice recordings                        |
| Models                              | ONNX                          | filesystem                         | ML weights resolved by manifest                     |

### 8.2 Path resolution

All paths are resolved via the `directories` crate so that platform conventions are honoured without hand-rolled logic:

| OS      | Recordings root                                                          |
| ------- | ------------------------------------------------------------------------ |
| macOS   | `~/Library/Application Support/NeuralPitch/recordings/`                  |
| Linux   | `$XDG_DATA_HOME/NeuralPitch/recordings/` (fallback `~/.local/share/...`) |
| Windows | `%APPDATA%\NeuralPitch\recordings\`                                      |

### 8.3 Recordings DB schema (Phase 2)

The schema is owned by `refinery` migrations under `crates/neural-pitch-core/src/store/migrations/`. The first migration is `V0001__init.sql` (refinery's required `V{n}__{desc}.sql` form). It creates the schema and writes an initial `schema_version` row.

The actual shipped schema diverges from the early-Phase-2 sketch above and is documented here verbatim — primary keys are 16-byte UUIDv7 BLOBs (not TEXT), tables are STRICT, and `analysis_cache` stores a postcard `BLOB` rather than `payload_json TEXT` (≈5–10× smaller for dense `Vec<F0Frame>`; see §13.3 wire-format note). The `recordings` row carries a soft-delete tombstone column plus two indexes (the second filters tombstones for the active-only fast path).

```sql
-- crates/neural-pitch-core/src/store/migrations/V0001__init.sql
CREATE TABLE schema_version (
  id          INTEGER PRIMARY KEY CHECK (id = 1),
  version     INTEGER NOT NULL
);
INSERT INTO schema_version (id, version) VALUES (1, 1);

CREATE TABLE recordings (
  id                    BLOB PRIMARY KEY,           -- UUIDv7, 16 bytes
  filename              TEXT    NOT NULL,
  created_at_unix_ms    INTEGER NOT NULL,
  duration_ms           INTEGER NOT NULL,
  sample_rate_hz        INTEGER NOT NULL,
  channels              INTEGER NOT NULL,
  bit_depth             INTEGER NOT NULL,
  format                TEXT    NOT NULL,           -- "flac" today
  a4_hz                 REAL    NOT NULL,
  instrument_profile    TEXT    NOT NULL,
  user_label            TEXT,
  deleted_at_unix_ms    INTEGER                     -- soft-delete tombstone
) STRICT;

CREATE INDEX idx_recordings_created_desc
  ON recordings(created_at_unix_ms DESC);
CREATE INDEX idx_recordings_live
  ON recordings(created_at_unix_ms DESC)
  WHERE deleted_at_unix_ms IS NULL;

CREATE TABLE analysis_cache (
  recording_id           BLOB    NOT NULL REFERENCES recordings(id) ON DELETE CASCADE,
  analyzer_name          TEXT    NOT NULL,
  analyzer_version       TEXT    NOT NULL,
  computed_at_unix_ms    INTEGER NOT NULL,
  result_format_version  INTEGER NOT NULL,
  result_blob            BLOB    NOT NULL,           -- postcard ContourResult
  PRIMARY KEY (recording_id, analyzer_name, analyzer_version)
) STRICT;
```

`PRAGMA journal_mode = WAL` is database-level and persists; it is set on every fresh connection (rather than in V0001) so existing DBs converge to WAL on next open. Connection-level pragmas (`synchronous = NORMAL`, `foreign_keys = ON`) are likewise issued by `RecordingsLibrary::new`.

`PRAGMA journal_mode = WAL` is database-level and persists. `PRAGMA synchronous = NORMAL` and `PRAGMA foreign_keys = ON` are connection-level and are issued by `open_library()` on every new connection rather than in the migration:

```rust
fn open_library(path: &Path) -> Result<Connection, Error> {
    let conn = Connection::open(path)?;
    conn.execute_batch("
        PRAGMA foreign_keys = ON;
        PRAGMA synchronous  = NORMAL;
    ")?;
    Ok(conn)
}
```

### 8.4 Settings migrations

`Settings` carries `schema_version: u32` and uses `#[serde(default)]` on every field. `Settings::default()` writes `schema_version: 1`. The `migrate()` function loops `from..SETTINGS_SCHEMA_VERSION`; the `migrate_v0_to_v1` arm exists only for users upgrading from an unreleased pre-`schema_version` build and may be removed once such users are no longer plausible.

### 8.5 Model resolver (Phase 2+)

The resolver (`scripts/fetch-models.sh` for offline use, plus an in-app variant) reads `models.toml`, downloads each entry to `<app-data>/models/<model-name>.onnx`, verifies the SHA-256, and atomically renames `<target>.partial` to `<target>`. The download path is guarded by an exclusive `fs2::flock` on `<target>.lock`.

Stale-lock recovery: on Windows, `fs2` flock locks are released when the file handle closes — a process crash auto-releases. On Linux/macOS, `fs2` uses `fcntl(F_OFD_*)` which is similarly handle-scoped. A janitor sweep on app startup removes any orphaned `<target>.partial` files older than a heuristic threshold.

On manifest parse error, the Settings UI Neural section displays a banner: "Model manifest unreadable; neural backends disabled. See logs at `<path>`." The error is logged at WARN with the underlying parse error.

## 9. Concurrency Model and Error Handling

### 9.1 Locked primitives

| Concern                           | Primitive                                                | Rationale                                                  |
| --------------------------------- | -------------------------------------------------------- | ---------------------------------------------------------- |
| Audio callback → DSP worker       | `rtrb` SPSC ring                                         | Wait-free; the only legal egress from the RT context.      |
| DSP worker → UI                   | `tauri::ipc::Channel<T>` (via `FrameSink` trait in core) | Ordered, per-listener; small fixed payloads.               |
| Fan-in from multiple offline jobs | `crossbeam-channel`                                      | MPMC; used only off the audio path.                        |
| Cross-runtime cancellation        | `tokio_util::CancellationToken`                          | Honoured by both tokio tasks and `std::thread` workers.    |
| Non-audio shared state            | `parking_lot::Mutex`                                     | Faster than `std::sync::Mutex`, non-poisoning.             |
| Async runtime                     | `tokio`                                                  | Tauri commands and HTTP only; never inside the DSP worker. |
| Long-lived workers                | `std::thread`                                            | DSP worker is a dedicated OS thread, not a tokio task.     |

### 9.2 Canonical worker startup pattern

```rust
// src-tauri/src/audio/worker.rs

pub struct DspWorkerHandle {
    join: Option<JoinHandle<()>>,
    heartbeat: Arc<AtomicU64>,
    drops: Arc<AtomicU64>,
}

pub struct DspController {
    inner: Mutex<DspControllerInner>,
}

struct DspControllerInner {
    worker: Option<DspWorkerHandle>,
    cancel: CancellationToken,
}

pub fn spawn_worker(
    mut estimator: Box<dyn PitchEstimator>,
    cons: rtrb::Consumer<f32>,
    sink: Box<dyn FrameSink<PitchFrame>>,
    cancel: CancellationToken,
) -> DspWorkerHandle {
    let window_size = estimator.config().window_size;
    let ring_capacity = (3 * window_size).next_power_of_two();
    debug_assert!(ring_capacity >= 3 * window_size);

    let heartbeat = Arc::new(AtomicU64::new(0));
    let drops = Arc::new(AtomicU64::new(0));
    let hb = heartbeat.clone();
    let dr = drops.clone();

    let join = std::thread::Builder::new()
        .name("neuralpitch-dsp".into())
        .spawn(move || {
            // ... drain `cons`, slide window by hop, call estimator.process,
            //     bump heartbeat, push frames into `sink`, honour `cancel` ...
        })
        .expect("dsp thread spawn failed");

    DspWorkerHandle { join: Some(join), heartbeat: hb, drops: dr }
}
```

The shell-side `FrameSink` impl wraps a `tauri::ipc::Channel<PitchFrame>`; the worker never names a Tauri type. Ring capacity is computed from the active `EstimatorConfig::window_size` per the rule in §6.4.

### 9.3 Recovery: device disconnect

A supervisor watches a sync `tokio::sync::mpsc::UnboundedReceiver<HealthEvent>` (cross-runtime: senders may be called from sync `std::thread` worker context via the channel's `send` method, which is non-blocking). The supervisor itself runs as a `tokio::task::spawn_blocking` shim if `recv_blocking` is needed, or as a normal tokio task if `recv().await` suffices — the design picks the second form because the receiver is async-native.

### 9.4 Worker thread panic (Day 1)

The DSP worker is a `std::thread` with a name. Day-1 recovery is a watchdog Tauri command (`check_dsp_health`) that inspects `JoinHandle::is_finished()` plus the heartbeat `AtomicU64`. If the heartbeat is stale or the join has finished unexpectedly, the supervisor surfaces a "DSP worker stopped — please restart capture" banner and offers a restart action. The handle is owned at the level of `DspController`; `JoinHandle::is_finished()` takes `&self` and is therefore safe to call through the controller's `Mutex<DspControllerInner>`.

### 9.5 Error type policy

Per ADR-0015 and P10:

- Library crates use a single `thiserror`-derived enum at the boundary.
- Application code uses `anyhow::Result<T>` with `.context(...)`.
- Tauri commands return `Result<T, String>` with `format!("{e:#}")` formatting.
- Audio callback uses atomic counters only.
- Tests are exempt from `unwrap_used`/`expect_used` lints.

## 10. Testing Strategy and TDD Harness

### 10.1 Tier structure

ADR-0016 locks the four-tier pyramid:

| Tier | Trigger                                                                                                                    | Phase introduced | Gating                          |
| ---- | -------------------------------------------------------------------------------------------------------------------------- | ---------------- | ------------------------------- |
| 1    | Synthesized signals (sine, vibrato, two-tone, noise, silence) + `proptest` + `frequency_to_note` golden table (MIDI 0–127) | Phase 0          | Every `cargo test`              |
| 2    | Philharmonia single-note voice fixtures in `tests/fixtures/`                                                               | Phase 1          | Every PR                        |
| 3    | Dataset slices via `scripts/fetch-test-data.sh` to gitignored `tests/data/`                                                | Phase 2          | `cargo test --features dataset` |
| 4    | Full benchmarks (MAESTRO, MUSDB18-HQ)                                                                                      | Release time     | Manual                          |

### 10.2 Phase-0 acceptance

- CI green on Linux/macOS/Windows × stable/beta.
- Tier-1 tests passing: golden-table round-trip, sine-at-known-Hz round-trip via `frequency_to_note`, windowing invariants via `proptest`.
- No Tauri UI in Phase 0.

### 10.3 Hardware-sanity check (Tier 1)

A tier-1 hardware test rig is documented in `tests/HARDWARE_RIG.md`: a known mic, a known sample rate, a known buffer size, and a known WebView version. Latency p50/p99 is measured against this rig; that is the test against which the Phase-1 acceptance bound (p50 ≤ 45 ms, p99 ≤ 70 ms) is evaluated.

### 10.4 TDD discipline

P4 mandates: every PR introducing or changing core behavior MUST land the failing test first (or in the same commit, with the test authored before the implementation). The pre-commit hook does not enforce this — the convention is human-enforced via PR review.

## 11. Observability

### 11.1 Logging

Per ADR-0017:

- `tracing` + `tracing-subscriber` for structured spans/events.
- Pretty-stderr formatter in dev (`init_pretty()` called early in `main`).
- JSON-rotating-file formatter in production (`init_json(app: &AppHandle)` called inside the Tauri `setup` hook, after the App is built; the log path is obtained from `app.path().app_log_dir()` because `tauri-plugin-log` does not expose a free path function before App construction).
- Per-frame analysis cache (the SQLite `analysis_cache` table, §8.3) is the persistent observability surface for offline jobs.
- No telemetry. No crash reports. No outbound traffic.

### 11.2 Audio-callback observability

Per P3: atomic counters only. The DSP worker reads `drops` and `heartbeat` and emits `tracing` events at most once per second.

### 11.3 Debug overlay

A dev-only `DebugFrame` payload (mel slice, YIN difference function, VAD state) is gated by the `debug-overlay` Cargo feature on `neural-pitch-core` (declared in §5.5) and a Tauri command `toggle_debug_overlay`. When the feature is off, the overlay is dead code. The frame is delivered via a binary `InvokeResponseBody::Raw` Channel (rather than JSON) because the difference-function `Vec<f32>` of size up to 2048 is too large to ship as JSON at frame rate.

## 12. Conventions and Enforcement (CI + Local + Convention)

ADR-0018 locks a triple-layer enforcement model: convention, pre-commit hooks, CI.

### 12.1 Workspace lint policy

```toml
[workspace.lints.rust]
unsafe_code  = "forbid"
missing_docs = "warn"

[workspace.lints.clippy]
pedantic     = { level = "warn",  priority = -1 }
unwrap_used  = "deny"
expect_used  = "deny"
panic        = "deny"
todo         = "warn"
```

`unsafe_code = "forbid"` is a hard, absolute ban. Per Rust language semantics, `forbid` cannot be relaxed by an inner `#![allow(unsafe_code)]` — a relaxation requires changing the workspace lint to `deny`. If a future feature genuinely needs `unsafe` (e.g. an FFI shim), the workspace lint is relaxed to `deny` and an ADR records the relaxation.

### 12.2 Commit format

Linux-kernel-style; enforced by a `commit-msg` hook:

```text
subsys: imperative subject under 72 chars

Body explains the *why*, not the *what*. Wrap at 72.
References to issues or prior commits go here.

Fixes: <hash> ("subject of broken commit")
Signed-off-by: Real Name <email@example.org>
```

The `commit-msg` script flags non-imperative first words (`*ed|*ing`) as a **fail**, accepting the false-positive cost on legitimate words like `feed`/`speed`. This is harder than a warn-only check and resolves the earlier mixed-signals on enforcement.

### 12.3 Pre-commit hooks

Installed via `scripts/install-hooks.sh`:

- `cargo fmt --check`
- `cargo clippy -D warnings`
- `cargo deny check` (when `deny.toml` is configured)
- `prettier`
- `eslint --max-warnings 0`
- `tsc --noEmit`
- trailing-whitespace, EOF-newline, large-file rejection
- `commit-msg` script (subject + DCO validation)

### 12.4 CI

GitHub Actions `ci.yml`:

- `commit-lint`
- `fmt`
- `lint` (clippy)
- `typecheck` (tsc)
- `test-matrix` (Linux/macOS/Windows × stable/beta)
- `deny` (`cargo-deny`; required only when `deny.toml` is populated, Phase 2+)
- `build` (Tauri bundle, smoke)

Branch protection requires `commit-lint`, `fmt`, `lint`, `typecheck`, all `test-matrix` cells, and `build`. `deny` is required only when `deny.toml` is populated (Phase 2+); branch protection is updated at that time.

### 12.5 DCO

Every commit carries `Signed-off-by:`; the pre-commit `commit-msg` hook and CI both verify the trailer.

## 13. Phased Roadmap with Acceptance Criteria

ADR-0009 locks phase ordering: ear-training (Phase 4) precedes stem separation (Phase 5).

### 13.1 Phase 0 — Skeleton

**Deliverables.** Workspace `Cargo.toml`; `crates/neural-pitch-core` with `PitchEstimator` trait (no impls yet), `NoteFormatter` trait + `EnglishFormatter`, `frequency_to_note` golden table; `src-tauri/` skeleton; CI Linux/macOS/Windows; pre-commit hooks; `LICENSE-MIT`/`LICENSE-APACHE`/`README.md`/`CONTRIBUTING.md`/`CHANGELOG.md`; commit-format hook.

**Acceptance.** CI green; Tier-1 tests passing; `cargo deny` clean (when configured); no Tauri UI required.

### 13.2 Phase 1 — Live monophonic tuner

**Deliverables.** `YinMpmEstimator` impl; `LiveTunerPipeline`; cpal capture + rtrb hand-off + DSP worker in `src-tauri/`; auto-prior via running F0 median + power-weighted F0 histogram; `tauri::ipc::Channel<PitchUpdate>` stream via `FrameSink` trait; React tuner page.

**Acceptance.** Octave-correctness ≥ 95% on Philharmonia voice fixtures with no manual instrument selection; mic-to-screen latency p50 ≤ 45 ms and p99 ≤ 70 ms on the Tier-1 hardware rig (§10.3); no audio dropouts at default buffer size on the rig; no through-monitoring.

**Status.** complete — closed 2026-06-03, commit `25b4b9c5b3612cb806316fe3d53b339c280343dd` (filled by the release script that runs `scripts/run-acceptance.sh`). See [`PHASE-1-CLOSEOUT.md`](./PHASE-1-CLOSEOUT.md).

### 13.3 Phase 2 — Recording, neural backend, vocal range

**Deliverables.** `RecorderPipeline` with `flacenc-rs` FLAC encoder (default) and `hound` WAV encoder (advanced opt-in); `rusqlite` + `refinery` recordings library + analysis cache; `models.toml` resolver with `reqwest`/rustls + `fs2` flock + janitor; PESTO `OnnxPesto` backend behind `feature = "neural"`; `PYinEstimator` behind `feature = "pyin"`; YAMNet auto-prior wrapper; vocal-range view; vibrato detector.

**Wire format.** `analysis_cache.result_blob` is a [`postcard`] 1.x byte stream of `analysis::contour::ContourResult` (≈5–10× smaller than `serde_json` for dense `Vec<F0Frame>`; the type is `serde::Serialize + Deserialize` regardless of build config so the cache survives feature flips). The blob's wire shape is keyed by `(recording_id, analyzer_name, analyzer_version)` — _any_ change to `ContourResult`'s field set or to a materially-impacting analyzer parameter (fmin/fmax defaults, hop/window, smoothing window, voicing threshold) MUST bump `analyzer_version` to invalidate prior rows. See `analysis::contour::PYIN_ANALYZER_VERSION` and `commands::DEFAULT_ANALYZER_VERSION` for the contributor invariant.

**Acceptance.** PESTO live tuner octave-correctness ≥ 97% with Viterbi decoding on the curated Philharmonia voice subset (the published PESTO MIR-1K reference is ~95–98% — 97% is the floor we hold ourselves to on a curated subset); recording → playback round-trip with no audible loss on FLAC; analysis cache hit on second open; no telemetry traffic except user-initiated model downloads.

**Sub-phase status.**

- **2.0 — Recording + FLAC + recordings SQLite library.** complete.
- **2.1 — Offline pYIN backend + analysis cache schema (postcard `ContourResult` blob).** complete.
- **2.2 — PESTO + CREPE-tiny estimators + shared Viterbi decoder, all behind `feature = "neural"` (ADR-0020); ONNX weights resolved at runtime, not bundled.** complete.
- **2.3 — Vocal range + vibrato detection.** pending.
- **2.4 — Recordings UI + wavesurfer.js + Phase-2 closeout.** pending.

### 13.4 Phase 3 — File upload, polyphonic transcription, MIDI export

**Deliverables.** `tauri-plugin-dialog` + `tauri-plugin-fs` (capability JSON updated to grant scoped read on user-picked paths); Basic Pitch ONNX integration; whole-mix polyphonic transcription; MIDI export via `midly` (SMF type 0 with 14-bit pitch bends); OpenSheetMusicDisplay notation view.

**Acceptance.** Basic Pitch transcription on a curated reference clip matches the published Basic Pitch metrics within reasonable export-fidelity drift; MIDI round-trips through a third-party DAW.

### 13.5 Phase 4 — Ear-training games

**Deliverables.** Movable-do solfege via a second `NoteFormatter` impl; karaoke pitch-ribbon view; SoundFont synthesis via `oxisynth` with a permissively-licensed default SF2 (FluidR3_GM under MIT); A4 honoured as a global cents detune at synthesis time, not by re-pitching samples.

**Acceptance.** Solfege drills run end-to-end; user-configured A4 reflected in synthesis pitch within ±5 cents.

### 13.6 Phase 5 — Stem separation

**Deliverables.** HTDemucs ONNX export; per-stem `PitchEstimator` dispatch; drum-onset detector (final crate selected at Phase 5 design pass).

**Acceptance.** 4-stem MUSDB18-HQ test SDR within 0.2 dB of the original PyTorch HTDemucs v4 checkpoint on the same test set — this is an export-fidelity test against the model's own reference, not a model-quality test against the published Demucs paper.

### 13.7 Phase 6 — Mobile

**Deliverables.** Tauri Mobile iOS + Android targets; `tract` fallback for inference where ONNX Runtime mobile bundles are too heavy.

**Acceptance.** Live tuner runs on iOS and Android with octave-correctness within 1% of desktop YIN on the same fixtures.

### 13.8 Phase 7 — Side-quests

**Deliverables.** Pure-Rust PESTO via burn / candle; voice-only fine-tune of PESTO trained on the author's personal corpus; OSS contributions upstream where appropriate. Phase 7 is explicitly exploratory; per-quest acceptance is captured in the issue tracker. A representative bound: a fine-tune is accepted when octave-correctness on a personal-voice held-out set exceeds baseline PESTO by ≥ 1%.

## 14. Cross-Cutting Risks

| Risk                                          | Surface                        | Mitigation                                                                                                                                                      |
| --------------------------------------------- | ------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `cpal` sole-maintainer                        | Real-time audio path           | Pin exact patch; vendor-and-patch posture; track issue #981; reserved fallback to platform-specific backends behind a future `crates/neural-pitch-io` boundary. |
| PESTO LGPL counsel review                     | Phase 2 default neural backend | If counsel rejects, default flips to `OnnxCrepeTiny` (MIT). Crate API does not change.                                                                          |
| ONNX Runtime mobile footprint                 | Phase 6                        | `tract` is the pre-declared fallback; `feature = "neural"` is sufficient surface to swap.                                                                       |
| Stack-table version skew                      | Build-time                     | Phase entry includes a "reconfirm versions" checklist; the recommended-stack open questions in §15 itemize the items to recheck.                                |
| Stale lock files from crashed model downloads | Persistence                    | Janitor sweep at app startup; OS-level handle release on crash.                                                                                                 |
| Worker thread panic                           | Concurrency                    | `JoinHandle::is_finished()` + heartbeat `AtomicU64` watchdog; `check_dsp_health` Tauri command; UI banner with restart action.                                  |

## 15. Open Questions and Deferred Decisions

The reviewer concerns at medium and low severity are absorbed here. Each entry names the section and the decision still owed.

- **Friends-and-contributors threshold for an installer code-signing budget.** The non-commercial framing means we do not pay for Apple Developer Program / Windows Authenticode certificates day 1. Whether to eat the ~$99/yr Apple fee at some later phase is deferred until first-launch friction becomes a real complaint.
- **Policy adjustments triggered by the secondary persona expanding beyond "people the author knows personally".** English-only strings, single library directory, visual-only feedback — revisit at the first external pull request.
- **`parking_lot::Mutex` vs `std::sync::Mutex` outside the audio path.** Defer until a contended lock shows up in benchmarks; both are forbidden in the audio callback either way.
- **Whether `MonitoringPipeline` ever ships** or remains a documented extension point. Defer to Phase 2 user feedback.
- **Whether `crates/neural-pitch-io` splits at Phase 2 (recording) or Phase 3 (file upload)**. Defer to whichever phase first introduces a second I/O consumer beyond `src-tauri/`.
- **`ndarray` major version compatibility with `ort` 2.0-rc.** Resolved at Phase 2 entry by reading `ort`'s then-current `Cargo.toml` and matching the major.
- **PESTO LGPL-3.0 redistribution posture.** Counsel sign-off required before any public binary release. Fallback is CREPE-tiny (MIT) via `yqzhishen/onnxcrepe`.
- **`tauri-plugin-fs` allowlist scope** for user-uploaded songs (Phase 3) vs a custom Tauri command path. Decision at Phase 3 design.
- **`apps/<binary>/` vs `src-tauri/`** for the Tauri member. Defer rename until a second binary target (CLI) joins the workspace.
- **Model distribution mechanism** (`hf_hub` runtime fetch, custom HTTPS+SHA256 fetcher, Git LFS). Defer to Phase 2 first model.
- **`CODE_OF_CONDUCT.md` on day 0.** Deferred until external contributors appear.
- **`AudioDecoder` trait shape.** Single-shot for day 1; streaming variant may be needed in Phase 6 for mobile.
- **`pitch/auto_prior.rs` YAMNet wrapper** (Phase 2): whether to expose raw class probabilities or only a coarse `InstrumentHint`. Defer to Phase 2.
- **Whether `Backend` should be `#[non_exhaustive]`.** Defer to the 0.x → 1.0 cut.
- **`ts-rs` vs `specta` for TS-side type generation.** Defer to Phase 1 entry.
- **Vite version pin** (5.x vs 6.x vs 7.x). Reconfirm at Phase 0 close against current `crates.io`/npm.
- **Stale-lock recovery janitor heuristic threshold** for `<target>.partial` cleanup. Defer to Phase 2 first model resolver implementation.
- **Drum onset detection crate** for Phase 5 — `aubio-rs` vs pure-Rust onset detector. Defer to Phase 5 design pass.

The medium-severity reviewer concerns explicitly absorbed here (rather than fixed inline) are: Phase-1 latency acceptance form (now stated as p50/p99); migration filename style (`V0001__init.sql`); `debug-overlay` feature listed in §5.5; settings-migration initial-write flow; PRAGMA placement (per-connection vs per-database); HTTP client crate selection (`reqwest` named in §3); cancellation example correctness (now constructs its own estimator with `&mut`); Channel binary mode for `DebugFrame`; supervisor task transport (`tokio::sync::mpsc`); Phase 3 plugin enumeration (`tauri-plugin-dialog` + `tauri-plugin-fs`); `tauri-plugin-log` path API (split init); PESTO acceptance bar (≥97%); frontend pitch-stream lifecycle; `unwrap`/`expect` example idiom; PESTO live-vs-offline status; worker JoinHandle ownership; SoundFont synth A4 handling; `make_estimator` model_root signature (`Option<&Path>`); core crate-type footnote; canonical design doc filename in §4; Vite version reconfirm; resolver stale-lock recovery; manifest-parse-error UX; commit-msg imperative-mood enforcement; Phase 7 done-criteria; solfege spelling consistency; branch-protection `deny` gating.

## 16. References

- [`../research/RESEARCH-REPORT.md`](../research/RESEARCH-REPORT.md) — DSP, ML, and ecosystem survey (background, library evaluation, latency budgets).
- [`../research/MODULAR-PITCH-RESEARCH.md`](../research/MODULAR-PITCH-RESEARCH.md) — modular pitch-pipeline research, trait surface, per-stem dispatch, auto-prior staging.
- [`../research/REPO-CONVENTIONS-REPORT.md`](../research/REPO-CONVENTIONS-REPORT.md) — repository conventions, idiomatic Rust patterns, hygiene baseline, FOSS norms.

## 17. ADR Index

The full ADR index lives at [`../adr/README.md`](../adr/README.md). The locked decisions referenced from this design are:

| ADR                                                                              | Title                                                                                  |
| -------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------- |
| [ADR-0001](../adr/0001-license-and-foss-posture.md)                              | License and FOSS posture                                                               |
| [ADR-0002](../adr/0002-mobile-ready-repo-and-crate-shape-day-1.md)               | Mobile-ready repo and crate shape day 1                                                |
| [ADR-0003](../adr/0003-frontend-stack-react-vite-ts-zustand-tailwind-shadcn.md)  | Frontend stack: React 19 + Vite + TS strict + Zustand + Tailwind + shadcn/ui           |
| [ADR-0004](../adr/0004-default-note-name-system-english-with-formatter-trait.md) | Default note-name system: English; multi-system formatter trait day 1                  |
| [ADR-0005](../adr/0005-a4-reference-configurable-default-440.md)                 | A4 reference: configurable day 1, default 440 Hz                                       |
| [ADR-0006](../adr/0006-visual-only-feedback-phase-1.md)                          | Visual-only feedback Phase 1; modular for monitoring later                             |
| [ADR-0007](../adr/0007-pitch-estimator-trait-and-auto-prior.md)                  | PitchEstimator trait + auto-prior; manual instrument selector demoted                  |
| [ADR-0008](../adr/0008-phase-1-yin-mpm-only-neural-phase-2.md)                   | Phase 1 ships YIN/MPM only; neural backends Phase 2                                    |
| [ADR-0009](../adr/0009-phase-ordering-ear-training-before-stem-separation.md)    | Phase ordering: ear-training before stem separation                                    |
| [ADR-0010](../adr/0010-audio-formats-wav-flac-mp3-day-1.md)                      | Audio formats: WAV+FLAC+MP3 day 1; Cargo-feature-gated additions                       |
| [ADR-0011](../adr/0011-recording-defaults-48k-24bit-mono-flac.md)                | Recording defaults: 48 kHz / 24-bit / mono / FLAC                                      |
| [ADR-0012](../adr/0012-recordings-library-and-analysis-cache-sqlite.md)          | Recordings library + per-recording analysis cache in SQLite                            |
| [ADR-0013](../adr/0013-settings-via-tauri-plugin-store.md)                       | Settings via tauri-plugin-store, separate from recordings DB                           |
| [ADR-0014](../adr/0014-concurrency-tokio-stdthread-rtrb.md)                      | Concurrency: tokio for Tauri/HTTP; std::thread for DSP worker; rtrb for audio boundary |
| [ADR-0015](../adr/0015-error-handling-thiserror-anyhow-no-panics.md)             | Error handling: thiserror in libs, anyhow in app, no panics in audio path              |
| [ADR-0016](../adr/0016-test-pyramid-tier-1-day-1.md)                             | Test pyramid: Tier 1 day 1; Tiers 2–4 phased                                           |
| [ADR-0017](../adr/0017-observability-tracing-tauri-plugin-log-no-telemetry.md)   | Observability: tracing + tauri-plugin-log; per-frame analysis cache; no telemetry      |
| [ADR-0018](../adr/0018-triple-layer-enforcement-convention-pre-commit-ci.md)     | Triple-layer enforcement: convention + pre-commit + CI                                 |
