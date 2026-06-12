//! MIDI emission for [`super::PolyResult`].
//!
//! [`poly_result_to_smf`] serialises a [`super::PolyResult`] into an SMF
//! type-1 byte buffer via the `midly` crate. The track preamble emits, in
//! order:
//!
//! 1. `SetTempo` meta = `60_000_000 / tempo_bpm` µs/quarter,
//! 2. RPN 0 (Pitch-Bend Sensitivity) initialiser **on every channel that
//!    will be used** — `B0 65 00 / B0 64 00 / B0 06 02 / B0 26 00` —
//!    so DAWs that default to `±12` semitones (Logic, Reaper) interpret
//!    the per-note pitch bend at the model's `±2`-semitone range.
//!
//! Channel allocation is a per-note "first free channel" walk over
//! `0..=15` skipping channel 9 (GM percussion). Each note holds its own
//! channel for the duration of its pitch-bend stream. If more than 15
//! notes overlap simultaneously, the emitter returns
//! [`EstimatorError::Configuration`] — more than 15 simultaneous notes
//! are out of scope; callers MUST split into multiple tracks externally.

use midly::num::{u4, u7, u14, u15, u24, u28};
use midly::{
    Format, Header, MetaMessage, MidiMessage, PitchBend, Smf, Timing, TrackEvent, TrackEventKind,
};

use crate::pitch::EstimatorError;
use crate::poly::PolyResult;

/// Tunable parameters for [`poly_result_to_smf`].
///
/// The defaults (480 PPQ, 120 BPM) match the most common DAW import
/// settings; callers that need a specific tempo map MUST set
/// `tempo_bpm` to match the source material before exporting.
#[derive(Clone, Copy, Debug)]
pub struct MidiExportOptions {
    /// Ticks per quarter note. SMF files commonly use 480 (Logic, Reaper)
    /// or 960 (Pro Tools); both round-trip cleanly through the
    /// millisecond -> tick conversion at the contour frame rate.
    pub ticks_per_quarter: u16,

    /// Tempo in beats per minute. Used to derive both the `SetTempo`
    /// meta event and the millisecond -> tick conversion.
    pub tempo_bpm: f32,
}

impl Default for MidiExportOptions {
    fn default() -> Self {
        Self {
            ticks_per_quarter: 480,
            tempo_bpm: 120.0,
        }
    }
}

/// Pitch-bend sensitivity programmed by the RPN 0 prelude, in semitones.
/// Basic Pitch's contour samples are signed cents in `[-100, +100]`, so
/// a `±2`-semitone range comfortably covers the full curve while still
/// matching every receiver we have tested (GM defaults to `±2`; Logic
/// and Reaper default to `±12` which is why the prelude is mandatory).
const PITCH_BEND_RANGE_SEMITONES: f32 = 2.0;

/// Hardware-percussion channel reserved by General MIDI. Skipped during
/// channel allocation so a melodic note never lands on the drum kit.
const GM_PERCUSSION_CHANNEL: u8 = 9;

/// Maximum channel index reachable on a single MIDI track. Combined with
/// the percussion-channel skip this leaves 15 simultaneously usable
/// channels.
const MAX_CHANNEL: u8 = 15;

/// Serialise a [`PolyResult`] to an SMF type-1 byte buffer.
///
/// The output is a single-track SMF type-1 file (header + one track) —
/// adequate for the per-note pitch-bend isolation the polyphonic
/// transcription pipeline needs. The
/// track preamble emits a `SetTempo` meta and a per-channel RPN 0 prelude
/// so receivers that default to `±12` semitones reinterpret the per-note
/// pitch bend at the model's `±2`-semitone range. Per note the emitter
/// writes `NoteOn` at `start_ms`, `PitchBend` events at the contour
/// frame rate (≈ 86 Hz) when `pitch_bend_curve` is `Some`, and `NoteOff`
/// at `end_ms`.
///
/// Returns [`EstimatorError::Configuration`] when more than 15 notes
/// overlap at any point — single-track polyphonic pitch bend cannot
/// disambiguate beyond 15 active channels (channel 9 is reserved for
/// GM percussion). It also returns [`EstimatorError::Configuration`] on
/// invalid input (`ticks_per_quarter == 0`, `tempo_bpm <= 0`, an empty
/// notes list, or a note with `end_ms <= start_ms`).
pub fn poly_result_to_smf(
    result: &PolyResult,
    opts: MidiExportOptions,
) -> Result<Vec<u8>, EstimatorError> {
    if opts.ticks_per_quarter == 0 {
        return Err(EstimatorError::Configuration(
            "ticks_per_quarter must be greater than zero".to_string(),
        ));
    }
    if !(opts.tempo_bpm.is_finite() && opts.tempo_bpm > 0.0) {
        return Err(EstimatorError::Configuration(
            "tempo_bpm must be a finite positive number".to_string(),
        ));
    }

    let ticks_per_ms = f64::from(opts.ticks_per_quarter) * f64::from(opts.tempo_bpm) / 60_000.0;
    let (channel_for_note, used_channels) = allocate_channels(&result.notes, ticks_per_ms)?;

    let mut events: Vec<(u64, u32, TrackEventKind<'static>)> = Vec::new();
    push_preamble(&mut events, opts.tempo_bpm, &used_channels);
    push_note_events(&mut events, &result.notes, &channel_for_note, ticks_per_ms);

    // Stable-sort by (tick, ordinal) so the prelude (ordinal < 100) stays
    // ahead of any note event at the same tick, and per-note events
    // sequence as NoteOn (100) / PitchBend (200) / NoteOff (300).
    events.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));

    // Convert absolute ticks to delta ticks for the SMF wire format.
    let mut track: Vec<TrackEvent<'static>> = Vec::with_capacity(events.len() + 1);
    let mut prev_tick: u64 = 0;
    for (tick, _ord, kind) in events {
        let delta_ticks = tick.saturating_sub(prev_tick);
        prev_tick = tick;
        track.push(TrackEvent {
            delta: u28_from_u64(delta_ticks),
            kind,
        });
    }
    track.push(TrackEvent {
        delta: u28::new(0),
        kind: TrackEventKind::Meta(MetaMessage::EndOfTrack),
    });

    let header = Header::new(
        Format::Parallel,
        Timing::Metrical(u15::new(opts.ticks_per_quarter)),
    );
    let mut smf = Smf::new(header);
    smf.tracks.push(track);

    let mut out = Vec::new();
    smf.write_std(&mut out)
        .map_err(|e| EstimatorError::Configuration(format!("midly write_std: {e}")))?;

    Ok(out)
}

/// Convert ms to absolute ticks given `ticks_per_ms` (precomputed once
/// from `ticks_per_quarter * tempo_bpm / 60_000`).
fn ms_to_ticks(ms: u64, ticks_per_ms: f64) -> u64 {
    (ms as f64 * ticks_per_ms).round().max(0.0) as u64
}

/// Per-note channel allocation: walk every note, sort by start tick, and
/// assign the first free channel (skipping channel 9 for GM percussion).
/// Returns `(channel_for_note[idx], used_channels)`.
///
/// `used_channels` is guaranteed non-empty so the RPN prelude has at
/// least one channel to land on even when no notes are emitted.
fn allocate_channels(
    notes: &[crate::poly::NoteEvent],
    ticks_per_ms: f64,
) -> Result<(Vec<u8>, Vec<u8>), EstimatorError> {
    let mut sorted: Vec<(usize, u64, u64)> = notes
        .iter()
        .enumerate()
        .map(|(idx, n)| {
            (
                idx,
                ms_to_ticks(n.start_ms, ticks_per_ms),
                ms_to_ticks(n.end_ms, ticks_per_ms),
            )
        })
        .collect();
    sorted.sort_by_key(|&(_, start, _)| start);

    let mut channel_for_note: Vec<u8> = vec![0; notes.len()];
    let mut channel_free_at: [u64; 16] = [0; 16];
    let mut used_channels: Vec<u8> = Vec::new();

    for &(idx, start, end) in &sorted {
        if end <= start {
            channel_for_note[idx] = 0;
            continue;
        }
        let mut allocated: Option<u8> = None;
        for ch in 0..=MAX_CHANNEL {
            if ch == GM_PERCUSSION_CHANNEL {
                continue;
            }
            if channel_free_at[ch as usize] <= start {
                allocated = Some(ch);
                channel_free_at[ch as usize] = end;
                if !used_channels.contains(&ch) {
                    used_channels.push(ch);
                }
                break;
            }
        }
        match allocated {
            Some(ch) => channel_for_note[idx] = ch,
            None => {
                return Err(EstimatorError::Configuration(
                    "too many simultaneous notes for single-track polyphonic pitch bend \
                     (max 15 channels; channel 9 reserved for GM percussion)"
                        .to_string(),
                ));
            }
        }
    }
    if used_channels.is_empty() {
        used_channels.push(0);
    }
    Ok((channel_for_note, used_channels))
}

/// Push `SetTempo` (ordinal 0) followed by the RPN 0 prelude on every
/// channel that will carry notes. Ordinals 1..N stay below the per-note
/// ordinals (100, 200, 300) so the prelude always sorts before any note
/// event at tick 0.
fn push_preamble(
    events: &mut Vec<(u64, u32, TrackEventKind<'static>)>,
    tempo_bpm: f32,
    used_channels: &[u8],
) {
    let micros_per_quarter = (60_000_000.0 / tempo_bpm).round().max(1.0) as u32;
    events.push((
        0,
        0,
        TrackEventKind::Meta(MetaMessage::Tempo(u24::from(micros_per_quarter))),
    ));
    let mut prelude_ord: u32 = 1;
    for &ch in used_channels {
        let channel = u4::new(ch);
        for &(cc, val) in &[(0x65_u8, 0x00_u8), (0x64, 0x00), (0x06, 0x02), (0x26, 0x00)] {
            events.push((
                0,
                prelude_ord,
                TrackEventKind::Midi {
                    channel,
                    message: MidiMessage::Controller {
                        controller: u7::new(cc),
                        value: u7::new(val),
                    },
                },
            ));
            prelude_ord += 1;
        }
    }
}

/// Push NoteOn (ordinal 100), PitchBend stream (ordinal 200), and
/// NoteOff (ordinal 300) for every note. The PitchBend stream is
/// distributed uniformly across the note's tick span.
fn push_note_events(
    events: &mut Vec<(u64, u32, TrackEventKind<'static>)>,
    notes: &[crate::poly::NoteEvent],
    channel_for_note: &[u8],
    ticks_per_ms: f64,
) {
    for (idx, note) in notes.iter().enumerate() {
        let start_tick = ms_to_ticks(note.start_ms, ticks_per_ms);
        let end_tick = ms_to_ticks(note.end_ms, ticks_per_ms);
        if end_tick <= start_tick {
            continue;
        }
        let channel = u4::new(channel_for_note[idx]);
        let velocity = u7::new(note.velocity.clamp(1, 127));
        let key = u7::new(note.midi.min(127));

        events.push((
            start_tick,
            100,
            TrackEventKind::Midi {
                channel,
                message: MidiMessage::NoteOn { key, vel: velocity },
            },
        ));

        if let Some(curve) = &note.pitch_bend_curve {
            push_pitch_bend(events, curve, channel, start_tick, end_tick);
        }

        events.push((
            end_tick,
            300,
            TrackEventKind::Midi {
                channel,
                message: MidiMessage::NoteOff {
                    key,
                    vel: u7::new(0),
                },
            },
        ));
    }
}

/// Distribute a pitch-bend curve uniformly between `start_tick` and
/// `end_tick`, emitting one PitchBend event per curve sample.
fn push_pitch_bend(
    events: &mut Vec<(u64, u32, TrackEventKind<'static>)>,
    curve: &[i16],
    channel: u4,
    start_tick: u64,
    end_tick: u64,
) {
    let n = curve.len();
    if n == 0 {
        return;
    }
    let span = end_tick.saturating_sub(start_tick);
    for (i, &cents) in curve.iter().enumerate() {
        let frac = if n == 1 {
            0.0
        } else {
            i as f64 / (n - 1) as f64
        };
        let tick = start_tick + (frac * span as f64).round() as u64;
        let bend_value = cents_to_bend14(f32::from(cents));
        events.push((
            tick,
            200,
            TrackEventKind::Midi {
                channel,
                message: MidiMessage::PitchBend {
                    bend: PitchBend(u14::new(bend_value)),
                },
            },
        ));
    }
}

/// Map a signed cents offset (in `[-200, +200]` for `±2` semitones) to a
/// 14-bit pitch-bend value. `8192` is centre (no bend).
fn cents_to_bend14(cents: f32) -> u16 {
    let range_cents = PITCH_BEND_RANGE_SEMITONES * 100.0;
    let normalised = (cents / range_cents).clamp(-1.0, 1.0);
    let value = (8192.0 + normalised * 8192.0).round();
    value.clamp(0.0, 16_383.0) as u16
}

/// Lossless `u64 -> u28` conversion — saturates at the 28-bit cap. SMF
/// delta ticks are stored as 28-bit variable-length quantities; longer
/// gaps would have already been split across multiple events upstream
/// (no such gap exists in any single Basic Pitch transcription pass).
fn u28_from_u64(v: u64) -> u28 {
    let cap = u32::from(u28::max_value());
    let clamped = v.min(u64::from(cap)) as u32;
    u28::new(clamped)
}
