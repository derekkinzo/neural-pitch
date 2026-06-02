# ADR-0010: Audio formats: WAV+FLAC+MP3 day 1; Cargo-feature-gated additions

## Status

Accepted — 2026-06-02.

## Context

The app reads audio in two contexts: file upload (Phase 3) and decode of recorded files (Phase 2 playback). The format menu spans WAV, FLAC, MP3, AAC/M4A, OGG/Vorbis, OGG/Opus, AIFF, ALAC. Each format Symphonia supports is gated behind a per-codec Cargo feature; enabling all of them bloats the Phase-0 binary with code that no one will exercise day 1.

## Decision

- Day 1 build supports **WAV, FLAC, and MP3** via Symphonia with default features only (the three are in Symphonia's default set).
- The `AudioDecoder` trait abstracts the decoder; `SymphoniaDecoder` is the day-1 impl.
- Additional formats (AAC, M4A, OGG/Vorbis, OGG/Opus, AIFF, ALAC) are added by app-level Cargo features (`feature = "aac"` etc.) wired through to Symphonia's per-codec features. They land when a concrete user need surfaces.
- The trait surface does not change as new formats are added.

## Consequences

- Phase-0 binary is slim.
- A user attempting to upload an unsupported format (Phase 3+) gets a clear error from the decoder layer rather than a silent corruption.
- The license register (`docs/licenses/REGISTER.md`) tracks Symphonia's MPL-2.0 status; adding new codec features may add additional licence entries.

## Alternatives Considered

- **Enable all Symphonia features day 1** — rejected for binary-size reasons.
- **Use ffmpeg via FFI** — rejected because it pulls a non-Rust runtime dependency, complicates mobile builds (Phase 6), and complicates the licence story.
- **Roll our own WAV decoder** — rejected because Symphonia is already pulled in for FLAC/MP3.
