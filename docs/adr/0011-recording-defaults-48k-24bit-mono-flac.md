# ADR-0011: Recording defaults: 48 kHz / 24-bit / mono / FLAC

## Status

Accepted — 2026-06-02.

## Context

Phase 2 introduces single-take voice recording. The recording-format defaults determine file size, fidelity, compatibility with downstream analysis (PESTO 48 kHz native), and the user-visible recording size on disk.

Possible defaults span:

- **Sample rate**: 44.1 kHz (CD), 48 kHz (OS default on macOS/Windows; PESTO native).
- **Bit depth**: 16-bit (CD), 24-bit (studio), 32-bit float (DAW).
- **Channels**: mono (analysis), stereo (capture).
- **Container**: WAV (uncompressed), FLAC (lossless ~50% smaller), MP3 (lossy).

## Decision

Default recording parameters:

- **48 kHz** — matches OS defaults, matches PESTO native, no resample on the analysis path.
- **24-bit** — avoids audible hiss at low input gain that 16-bit would clip into; voice is dynamic and headroom matters.
- **Mono** — analysis is mono; stereo capture is summed at decode for analysis, so storing stereo doubles file size for no analysis benefit.
- **FLAC** — lossless, ~50% the size of WAV, broadly playable; default container.

Advanced settings allow override (sample rate from device list; 16-/24-/32-bit; mono/stereo; FLAC/WAV).

## Consequences

- A typical 5-minute single-take vocal recording is approximately 30–45 MB rather than 60–90 MB (WAV).
- The FLAC encoder is `flacenc-rs` (pure-Rust); the WAV encoder is `hound` (advanced opt-in only).
- Re-analysis years later runs against the same sample rate the analysis was originally tuned for.
- Stereo capture for instrumental recordings is one advanced-settings click away.

## Alternatives Considered

- **44.1 kHz default** — rejected because PESTO is 48 kHz native and OS defaults are 48 kHz on the platforms we target.
- **16-bit default** — rejected because voice is dynamic and 16-bit clips audibly at low gain.
- **WAV default** — rejected because the file-size cost is significant for a recording-heavy workflow and FLAC is universally readable.
- **MP3 default** — rejected because it is lossy and re-analysis would inherit lossy artefacts.
