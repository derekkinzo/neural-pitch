# ADR-0014: Concurrency: tokio for Tauri/HTTP; std::thread for DSP worker; rtrb for audio boundary

## Status

Accepted — 2026-06-02.

## Context

The app has three concurrency contexts with very different requirements:

1. **The cpal audio callback** — real-time, no allocation, no locks, no syscalls.
2. **The DSP worker** — long-lived, owns the pitch estimator, drains the audio ring, emits per-frame outputs.
3. **The Tauri command surface and HTTP fetches** — async, cooperative, cancellation-aware.

Trying to run all three on tokio would force the audio callback through async machinery (forbidden) or force the DSP worker to interact with tokio in a way that costs more than it earns. Trying to run all three on `std::thread` would lose tokio's cooperative cancellation that Tauri commands rely on.

## Decision

- **`rtrb` SPSC ring buffer** — the *only* legal egress from the audio callback. Wait-free, allocation-free, single-producer single-consumer fits the cpal-callback → DSP-worker hand-off.
- **`std::thread`** — the DSP worker is a dedicated, named OS thread. Long-lived; never destroyed except on shutdown.
- **`tokio` (1.x, multi-thread runtime)** — Tauri commands and HTTP fetches (model downloads). Tokio is not used inside the DSP worker.
- **`tauri::ipc::Channel<T>`** — the DSP-to-UI per-frame stream. Lives in the shell crate; surfaced to the worker via the `FrameSink` trait defined in `neural-pitch-core`.
- **`tokio_util::CancellationToken`** — cooperative cancellation that crosses the runtime boundary; honoured by both tokio tasks and `std::thread` workers.
- **`parking_lot::Mutex`** — shared state outside the audio callback. Faster than `std::sync::Mutex`, non-poisoning. Forbidden in the audio callback.
- **`crossbeam-channel`** — MPMC fan-in from multiple offline jobs to a single supervisor. Used off the audio path only.

The audio callback never logs and never allocates; observability from the callback is via `AtomicU64` counters that the DSP worker reads and emits.

## Consequences

- The audio callback is genuinely RT-safe — auditable rules with auditable enforcement.
- DSP worker startup is a `std::thread::Builder::new().name("neuralpitch-dsp").spawn(...)`; thread names show up in profilers.
- Tauri commands are async and can be awaited; cancellation is uniform across the codebase.
- The DSP worker can be told to stop via the cancellation token without any back-channel.

## Alternatives Considered

- **All-tokio** — rejected because the audio callback must not run inside the tokio runtime.
- **All-`std::thread`** — rejected because Tauri commands rely on async; rebuilding a custom event loop would be more work for the same outcome.
- **`async-channel` for audio** — rejected because the callback cannot await.
- **`flume` for audio** — rejected because rtrb is the survey-recommended SPSC primitive for RT audio in Rust.
