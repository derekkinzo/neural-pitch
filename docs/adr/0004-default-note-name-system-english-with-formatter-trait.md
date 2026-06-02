# ADR-0004: Default note-name system: English; multi-system formatter trait day 1

## Status

Accepted — 2026-06-02.

## Context

The app must display note names. Western music has multiple naming conventions: English (C D E F G A B), German (replaces B with H, B♭ with B), fixed-do solfege (Do Re Mi Fa Sol La Si — fixed pitches), and movable-do solfege (Do Re Mi … relative to a tonal centre, used in ear-training pedagogy).

The author's primary use case (singing-voice tuner) is well served by English. Phase 4 (ear-training) explicitly needs movable-do solfege as a teaching tool. Designing this as a string-replacement table after the fact would require modifying every component that renders a note name.

## Decision

- A `NoteFormatter` trait exists in `crates/neural-pitch-core/src/music/format.rs` from day 1.
- Day 1 implementation is `EnglishFormatter` (C D E F G A B with `#`/`b` accidentals).
- The trait surface accepts a MIDI integer and an octave-system parameter, returning a display string.
- The internal data model is **MIDI numbers always**. Note names are a presentation concern at the formatter boundary.
- Movable-do solfege is added as a second `NoteFormatter` impl in Phase 4.
- The chosen formatter is a user-settable preference (Phase 4); Phase 1 ships English-only and the setting is hidden.

## Consequences

- Adding a new note-name system in any future phase is mechanical: implement the trait, add the variant to the user-settings enum, ship a new locale string for the setting label.
- Internal logging, error messages, and audit trails always use MIDI integers and Hertz values; never localised note names.
- Storage formats (recordings DB, analysis cache) never contain localised note names.

## Alternatives Considered

- **English-only with no trait** — rejected because Phase 4 ear-training requires solfege, and adding the abstraction later is more disruptive than adding it day 1.
- **Localised strings via `react-intl` / `i18next`** — rejected for day 1 because the persona scope is a single English-speaking author plus known friends; full i18n infrastructure is deferred.
- **Fixed-do solfege as the day-1 default** — rejected because the primary persona uses English notation in their existing musical practice.
