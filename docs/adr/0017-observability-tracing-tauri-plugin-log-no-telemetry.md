# ADR-0017: Observability: tracing + tauri-plugin-log; per-frame analysis cache; no telemetry

## Status

Accepted — 2026-06-02.

## Context

The app needs two kinds of observability:

1. **Logs** — structured, with span context, written somewhere a user (or developer-friend) can find them when something goes wrong.
2. **Persistent per-frame outputs** — for offline analysis (vocal range, vibrato), the analyser output is itself a kind of observability that the user inspects.

What it does _not_ need: telemetry, crash reports, analytics. The FOSS posture (ADR-0001) is local-only.

## Decision

- **`tracing` + `tracing-subscriber`** for structured spans/events. Pretty-stderr formatter in dev (`init_pretty()` called early in `main`); JSON-rotating-file formatter in production (`init_json(app: &AppHandle)` called inside the Tauri `setup` hook, after the App is built — the production log path is obtained from `app.path().app_log_dir()` because `tauri-plugin-log` does not expose a free path function before App construction).
- **`tauri-plugin-log`** owns the OS-blessed log file paths.
- **Per-frame analysis cache** (the SQLite `analysis_cache` table from ADR-0012) is the persistent observability surface for offline jobs.
- **No telemetry, ever, except explicit user-initiated model downloads.**
- **No crash reporting.**
- **Live debug overlay** behind a `debug-overlay` Cargo feature on `neural-pitch-core`. When the feature is off, the overlay is dead code. The `DebugFrame` payload (mel slice, YIN difference function, VAD state) is delivered via a binary `InvokeResponseBody::Raw` Channel because the difference-function `Vec<f32>` is too large for JSON at frame rate.
- **Audio callback emits via atomic counters only.** No `tracing::*` from the callback; the DSP worker reads the counters and emits structured events from non-RT context.

## Consequences

- A friend reporting a bug can attach a log file path that the developer-author can read.
- No data ever leaves the user's machine without explicit consent (a model-download click).
- Adding telemetry later requires a user-visible opt-in toggle and a settings entry.
- The dev debug overlay is a fast iteration tool and never ships in production builds.

## Alternatives Considered

- **Sentry / similar crash reporter** — rejected because of the no-telemetry posture.
- **`log` instead of `tracing`** — rejected because `tracing`'s span model is materially better for async code.
- **Logs to stdout only** — rejected because Tauri-bundled apps on Windows have no stdout terminal by default; users cannot retrieve logs.
- **Custom per-frame log file** — rejected because the SQLite analysis cache already serves the persistent-output role.
