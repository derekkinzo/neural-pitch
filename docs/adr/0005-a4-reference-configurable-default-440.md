# ADR-0005: A4 reference: configurable day 1, default 440 Hz

## Status

Accepted — 2026-06-02.

## Context

Pitch-to-note-name conversion depends on a reference frequency for A4 (the A above middle C). The modern standard is 440 Hz, but historical and contemporary practice covers a range:

- 415 Hz — Baroque
- 430 Hz — Classical
- 435 Hz — late-19th-century French
- 440 Hz — modern standard
- 442 Hz — common European orchestral tuning
- 443 Hz — common Berlin/Vienna tuning
- 466 Hz — chorton

Singers practising historical repertoire need a non-440 reference. The cost of supporting this day 1 is a single `f32` parameter on `frequency_to_note` and a settings field; the cost of retrofitting later is touching every conversion site.

## Decision

- A4 reference is configurable from day 1.
- Default is 440 Hz.
- A preset list is offered: 415, 430, 435, 440, 442, 443, 466.
- A freeform input accepts any value in 410–470 Hz.
- A4 is **always passed as a parameter** to public functions in `neural-pitch-core`. There is no module-level mutable A4 state.
- A4 is saved per-recording at capture time so that re-analysis years later honours the tuning the user was practising at.
- Synth playback (Phase 4 SoundFont) honours the same setting — the SoundFont is not re-pitched; A4-offset is applied as a global cents detune at synthesis time.

## Consequences

- Every public function in `music::note` carries an `a4_hz: f32` parameter; this is verbose but keeps the API honest about what it depends on.
- The recordings DB schema (ADR-0012) includes an `a4_hz REAL NOT NULL` column.
- The settings UI exposes an A4 control in advanced settings.
- Phase 4 synth quality depends on the SoundFont used; cents-detune is acceptable for ear-training drills but would not be for high-fidelity playback.

## Alternatives Considered

- **Hard-code 440 Hz** — rejected because Phase 4 ear-training and the secondary persona's historical-music interest both require alternative tunings.
- **Module-level mutable A4** — rejected for thread-safety reasons and because it makes pure-function contracts impure.
- **Configurable but no per-recording capture** — rejected because the use case "I changed my A4 setting after recording, please re-analyse correctly" requires storing the capture-time A4 with the recording.
