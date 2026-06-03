# ADR-0015: Error handling: thiserror in libs, anyhow in app, no panics in audio path

## Status

Accepted — 2026-06-02.

## Context

The app spans three error-handling contexts:

1. **Library crates** (`neural-pitch-core`, future `-io`, `-ml`) — public API surface; callers should be able to `match` on the error variant.
2. **Application code** (`src-tauri/`) — gluing things together; callers care about _context_ (what was being attempted) more than the specific variant.
3. **Audio callback** — real-time; must not panic, must not allocate, must not block.

A single error-handling style across all three either burdens the application code with `From` boilerplate or leaves library callers without typed errors.

## Decision

- **Library crates use `thiserror`-derived enums.** One `Error` enum per crate at the library boundary. Variants carry typed payload where useful.
- **Application code uses `anyhow::Result<T>`** with `.context(...)` on bubble-up. Provides a chain of context strings that show up in logs.
- **Tauri commands return `Result<T, String>`** with `format!("{e:#}")` formatting (`#` triggers anyhow's full chain).
- **Audio callback uses atomic counters only.** Drops increment `AtomicU64`; the DSP worker reads the counter and emits structured `tracing` events from a non-RT context.
- **Tests are exempt from `unwrap_used`/`expect_used` lints** so test failures surface immediately with full panic context.
- **Production paths return `Result` everywhere.** `clippy::unwrap_used` and `clippy::expect_used` are denied workspace-wide.

## Consequences

- Library callers can pattern-match on `EstimatorError::ModelNotFound(_)` and react meaningfully.
- Application errors carry breadcrumbs ("loading model", "starting capture", "writing analysis cache") all the way to the UI.
- The audio callback is auditable for absence of panic-prone constructs.
- Tests are loud and concise; production is quiet and graceful.

## Alternatives Considered

- **`anyhow` everywhere** — rejected because library callers lose the ability to match on variants.
- **`thiserror` everywhere** — rejected because application code becomes overwhelmed with `From` impls and per-call-site enum variants.
- **`Result<T, Box<dyn Error>>`** — rejected because it loses the `Display`-with-chain ergonomics anyhow provides.
- **Allow `unwrap` / `expect` in production with discipline** — rejected because clippy enforcement is strictly cheaper than human review for a clear rule like "no panic in production".
