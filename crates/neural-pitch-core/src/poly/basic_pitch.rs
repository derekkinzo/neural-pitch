//! Basic Pitch v1 polyphonic transcription estimator.
//!
//! Wraps the Spotify Basic Pitch v1 ONNX graph (Apache-2.0) behind an
//! `ort::Session` and a `rubato` resampler. The pipeline is:
//!
//! 1. Mono-sum + resample to 22.05 kHz (zero-copy fast path when the
//!    input is already 22.05 kHz).
//! 2. Window at `AUDIO_N_SAMPLES = 43_844` with `OVERLAP_FRAMES = 30`
//!    (Issue #190 frame-drift mitigation).
//! 3. Run a single ORT session per window returning `contour`
//!    `[1, 172, 264]`, `note` `[1, 172, 88]`, and `onset`
//!    `[1, 172, 88]`. Stitch with `TRIM_FRAMES = 15` per side of every
//!    interior window.
//! 4. Heuristic note assembly with the Spotify default thresholds.
//! 5. Pitch-bend curve sampling from the contour head's three-bin
//!    neighbourhood centred on each accepted note.
//!
//! The bundled ONNX uses TensorFlow's `serving_default_*` /
//! `StatefulPartitionedCall:*` tensor names (the saved-model export);
//! we resolve them by inspecting the session's `inputs()` / `outputs()`
//! at construction time so future re-exports with different naming
//! conventions still work without code changes.

use std::path::Path;

use rubato::{FftFixedIn, Resampler};

use crate::analysis::viterbi::{TransitionModel, decode};
use crate::pitch::EstimatorError;
use crate::poly::{NoteEvent, PolyEstimator, PolyResult};

/// Bundled ONNX bytes — pinned at compile time via `include_bytes!`.
/// Bundled bytes survive cargo packaging untouched; the file lives at
/// `crates/neural-pitch-core/assets/basic_pitch_v1.0.onnx`.
const BASIC_PITCH_ONNX: &[u8] = include_bytes!("../../assets/basic_pitch_v1.0.onnx");

/// Native sample rate of the Basic Pitch model, in Hertz.
const BASIC_PITCH_SR_HZ: u32 = 22_050;
/// Samples per inference window — the model's fixed input length.
const AUDIO_N_SAMPLES: usize = 43_844;
/// Hop size between successive output frames within a window, in
/// 22.05 kHz samples. Drives the model's native output frame rate
/// (≈ 86.13 Hz at 22.05 / 256).
const FFT_HOP: usize = 256;
/// Number of output frames the model emits per window.
const FRAMES_PER_WINDOW: usize = 172;
/// Overlap between consecutive inference windows, in output frames.
/// Half is trimmed from each interior side at stitching time per the
/// upstream Issue #190 frame-drift mitigation.
const OVERLAP_FRAMES: usize = 30;
/// Frames trimmed from each interior side of every window during
/// stitching. The first window keeps its leading edge, the last window
/// keeps its trailing edge.
const TRIM_FRAMES: usize = OVERLAP_FRAMES / 2;
/// Hop between consecutive inference windows, in 22.05 kHz samples.
/// `AUDIO_N_SAMPLES - OVERLAP_FRAMES * FFT_HOP = 36_164`.
const WINDOW_HOP_SAMPLES: usize = AUDIO_N_SAMPLES - OVERLAP_FRAMES * FFT_HOP;
/// Number of pitch bins emitted by the `note` and `onset` heads.
/// Basic Pitch covers MIDI 21..=108 inclusive — the 88 piano keys.
const N_PITCH_BINS: usize = 88;
/// Lowest MIDI note covered by the `note` / `onset` heads. Bin 0 is
/// MIDI 21 (A0).
const MIDI_OFFSET: u8 = 21;
/// Number of contour bins. 3 bins per semitone × 88 semitones.
const N_CONTOUR_BINS: usize = N_PITCH_BINS * 3;
/// Number of consecutive sub-threshold onset frames required before a
/// second onset crossing is treated as a *new* note rather than a
/// continuation of the existing one. Keeps a broad onset activation
/// (typically 2-4 frames wide on attacks) from splitting every attack
/// into a series of single-frame notes.
const ONSET_RESET_FRAMES: usize = 4;

/// Thresholds and durations driving the heuristic note-assembly stage.
///
/// Defaults mirror Spotify's `note_creation.py` upstream: an onset
/// posterior above `0.5` opens a candidate, the candidate stays open
/// while the per-frame `note` posterior is `>= 0.3`, and closes after
/// `max_silent_frames` consecutive frames below threshold. Notes shorter
/// than `min_note_frames` are discarded.
#[derive(Clone, Debug)]
pub struct NoteAssemblyConfig {
    /// Onset posterior required to open a new note candidate.
    pub onset_threshold: f32,

    /// Per-frame `note` posterior required to keep a note open.
    pub frame_threshold: f32,

    /// Minimum note length, in analysis frames (≈ 11.61 ms each at the
    /// Basic Pitch v1 frame rate).
    pub min_note_frames: usize,

    /// Number of consecutive sub-threshold frames that closes a note.
    pub max_silent_frames: usize,
}

impl Default for NoteAssemblyConfig {
    fn default() -> Self {
        Self {
            onset_threshold: 0.5,
            frame_threshold: 0.3,
            min_note_frames: 11,
            max_silent_frames: 2,
        }
    }
}

/// Polyphonic transcription estimator backed by Spotify's Basic Pitch v1
/// ONNX (Apache-2.0).
///
/// Construction is path-based to match the existing CREPE ergonomics
/// in [`crate::pitch`]. The ONNX session is allocated in the
/// constructor; [`super::PolyEstimator::analyze`] re-builds the
/// resampler each call because the input length is variable but the
/// scratch buffers sit on the stack-local heap and the cost is small
/// relative to ORT inference.
pub struct BasicPitchEstimator {
    /// Heuristic note-assembly thresholds.
    assembly: NoteAssemblyConfig,
    /// Loaded ONNX session — boxed so the public surface does not leak
    /// the `ort` 2.0 type names into downstream callers.
    session: Box<ort::session::Session>,
    /// Resolved input tensor name (e.g. `serving_default_input_2:0` for
    /// the bundled ONNX). Captured at construction time so per-call
    /// inference does not need to re-walk the session metadata.
    input_name: String,
    /// Resolved output tensor names in `(contour, note, onset)` order.
    /// Basic Pitch's saved-model export emits them as
    /// `StatefulPartitionedCall:0` / `:1` / `:2` respectively.
    output_names: [String; 3],
}

impl BasicPitchEstimator {
    /// Build an estimator from a `basic_pitch_v1.0.onnx` file on disk.
    pub fn from_onnx(path: &Path) -> Result<Self, EstimatorError> {
        if !path.exists() {
            return Err(EstimatorError::ModelNotFound(path.to_path_buf()));
        }
        let bytes = std::fs::read(path)
            .map_err(|e| EstimatorError::Ort(format!("read basic-pitch onnx: {e}")))?;
        Self::from_bytes(&bytes)
    }

    /// Build an estimator from the bundled `basic_pitch_v1.0.onnx`.
    pub fn from_bundled() -> Result<Self, EstimatorError> {
        Self::from_bytes(BASIC_PITCH_ONNX)
    }

    /// Shared constructor entry point — accepts the raw ONNX bytes and
    /// builds the session + resolves input/output names.
    fn from_bytes(bytes: &[u8]) -> Result<Self, EstimatorError> {
        let session = ort::session::Session::builder()
            .map_err(|e| EstimatorError::Ort(format!("session builder: {e}")))?
            .commit_from_memory(bytes)
            .map_err(|e| EstimatorError::Ort(format!("commit_from_memory: {e}")))?;

        // Resolve input / output names from the session metadata. The
        // bundled ONNX has exactly one input and three outputs; we pin
        // the order by tensor-shape signature so a re-export with
        // different `StatefulPartitionedCall:N` indexes still maps onto
        // (contour, note, onset).
        let input_name = session
            .inputs()
            .first()
            .map(|i| i.name().to_string())
            .ok_or_else(|| EstimatorError::Ort("basic-pitch onnx has no inputs".to_string()))?;

        // Resolve output names by tensor shape *and* a stable name-suffix
        // tiebreak. The Basic Pitch SavedModel export uses
        // `StatefulPartitionedCall:0` for the 264-bin contour, `:1` for
        // the 88-bin note (sustain) head, and `:2` for the 88-bin onset
        // head (verified empirically via `examples/inspect_basic_pitch`).
        // Ort's `session.outputs()` iteration order is not guaranteed to
        // match the saved-model order, so we key on the suffix.
        let mut contour_name: Option<String> = None;
        let mut note_name: Option<String> = None;
        let mut onset_name: Option<String> = None;
        let mut other_88: Vec<String> = Vec::new();
        for outlet in session.outputs() {
            let dtype_dbg = format!("{:?}", outlet.dtype());
            let nm = outlet.name().to_string();
            if dtype_dbg.contains("264") {
                contour_name = Some(nm);
            } else if dtype_dbg.contains(", 88]") || dtype_dbg.contains("88]") {
                if nm.ends_with(":1") {
                    note_name = Some(nm);
                } else if nm.ends_with(":2") {
                    onset_name = Some(nm);
                } else {
                    other_88.push(nm);
                }
            }
        }
        // Fallback: if the suffix tiebreak did not resolve note/onset
        // (e.g. a re-export with non-standard names), keep the first
        // 88-bin output as note and the second as onset.
        if note_name.is_none() && !other_88.is_empty() {
            note_name = Some(other_88.remove(0));
        }
        if onset_name.is_none() && !other_88.is_empty() {
            onset_name = Some(other_88.remove(0));
        }
        let contour_name = contour_name.ok_or_else(|| {
            EstimatorError::Ort("basic-pitch onnx missing 264-bin contour output".to_string())
        })?;
        let note_name = note_name.ok_or_else(|| {
            EstimatorError::Ort("basic-pitch onnx missing 88-bin note output".to_string())
        })?;
        let onset_name = onset_name.ok_or_else(|| {
            EstimatorError::Ort("basic-pitch onnx missing 88-bin onset output".to_string())
        })?;
        Ok(Self {
            assembly: NoteAssemblyConfig::default(),
            session: Box::new(session),
            input_name,
            output_names: [contour_name, note_name, onset_name],
        })
    }
}

/// Mono-sum the input down to a single channel and resample to the
/// target rate. Returns the 22.05 kHz buffer ready for windowing. The
/// input is taken as mono PCM per the [`PolyEstimator::analyze`]
/// contract; this helper exists so future stereo support is a one-line
/// change.
fn resample_to_basic_pitch(audio: &[f32], sample_rate_hz: u32) -> Result<Vec<f32>, EstimatorError> {
    if sample_rate_hz == BASIC_PITCH_SR_HZ {
        return Ok(audio.to_vec());
    }
    if audio.is_empty() {
        return Ok(Vec::new());
    }
    if sample_rate_hz == 0 {
        return Err(EstimatorError::Configuration(
            "sample_rate_hz must be greater than zero".to_string(),
        ));
    }

    // FftFixedIn is the simplest rubato resampler that handles arbitrary
    // sample-rate ratios with a fixed input chunk size. We pick a
    // 4_096-sample chunk so the FFT cost per call is bounded; the final
    // partial chunk is flushed via process_partial.
    let chunk_size_in: usize = 4_096;
    let mut resampler = FftFixedIn::<f32>::new(
        sample_rate_hz as usize,
        BASIC_PITCH_SR_HZ as usize,
        chunk_size_in,
        2,
        1,
    )
    .map_err(|e| EstimatorError::Configuration(format!("rubato construct: {e}")))?;

    let mut out: Vec<f32> = Vec::with_capacity(
        audio.len() * (BASIC_PITCH_SR_HZ as usize) / (sample_rate_hz as usize) + chunk_size_in,
    );
    let mut idx: usize = 0;
    while idx + chunk_size_in <= audio.len() {
        let waves_in: [&[f32]; 1] = [&audio[idx..idx + chunk_size_in]];
        let waves_out = resampler
            .process(&waves_in, None)
            .map_err(|e| EstimatorError::Ort(format!("rubato process: {e}")))?;
        out.extend_from_slice(&waves_out[0]);
        idx += chunk_size_in;
    }
    if idx < audio.len() {
        let waves_in: [&[f32]; 1] = [&audio[idx..]];
        let waves_out = resampler
            .process_partial(Some(&waves_in), None)
            .map_err(|e| EstimatorError::Ort(format!("rubato process_partial: {e}")))?;
        out.extend_from_slice(&waves_out[0]);
    }
    Ok(out)
}

/// One window's worth of model output, after we have moved it out of the
/// ORT-owned tensor and into a contiguous host-side `Vec`.
struct WindowOutput {
    /// `[FRAMES_PER_WINDOW][N_PITCH_BINS]` onset posteriors.
    onset: Vec<[f32; N_PITCH_BINS]>,
    /// `[FRAMES_PER_WINDOW][N_PITCH_BINS]` note (sustain) posteriors.
    note: Vec<[f32; N_PITCH_BINS]>,
    /// `[FRAMES_PER_WINDOW][N_CONTOUR_BINS]` contour activations.
    contour: Vec<Vec<f32>>,
}

/// Run a single window's inference and copy the three outputs out of
/// the ORT session before the borrow ends. Allocations here are
/// constructor-time-bounded; the per-call cost is dominated by the ONNX
/// forward pass.
fn run_window(
    estimator: &mut BasicPitchEstimator,
    window: &[f32],
) -> Result<WindowOutput, EstimatorError> {
    if window.len() != AUDIO_N_SAMPLES {
        return Err(EstimatorError::Configuration(format!(
            "basic-pitch window length {} != {AUDIO_N_SAMPLES}",
            window.len()
        )));
    }
    // Build the [1, AUDIO_N_SAMPLES, 1] input tensor. We hold the
    // samples in a flat slice and let `TensorRef::from_array_view`
    // bind them to the session via the `(shape, &[T])` adaptor so we
    // never have to depend on `ort`'s internal `ndarray` version.
    let shape: [usize; 3] = [1, AUDIO_N_SAMPLES, 1];
    let input_value = ort::value::TensorRef::from_array_view((shape, window))
        .map_err(|e| EstimatorError::Ort(format!("tensor view: {e}")))?;
    let inputs: Vec<(
        std::borrow::Cow<'_, str>,
        ort::session::SessionInputValue<'_>,
    )> = vec![(
        std::borrow::Cow::Borrowed(estimator.input_name.as_str()),
        ort::session::SessionInputValue::from(input_value),
    )];
    let outputs = estimator
        .session
        .run(inputs)
        .map_err(|e| EstimatorError::Ort(format!("session run: {e}")))?;

    let contour = extract_2d(&outputs, &estimator.output_names[0], N_CONTOUR_BINS)?;
    let note_raw = extract_2d(&outputs, &estimator.output_names[1], N_PITCH_BINS)?;
    let onset_raw = extract_2d(&outputs, &estimator.output_names[2], N_PITCH_BINS)?;

    // Convert the variable-width Vec<Vec<f32>> for the 88-bin heads into
    // fixed-width [f32; 88] rows so downstream stitching is cache-friendly.
    let onset = pack_88(&onset_raw)?;
    let note = pack_88(&note_raw)?;

    Ok(WindowOutput {
        onset,
        note,
        contour,
    })
}

/// Extract a 3-D float tensor named `name` from `outputs`, drop the
/// leading batch axis, and return one row per frame as a contiguous
/// `Vec<Vec<f32>>` of shape `[T][trailing]`.
fn extract_2d(
    outputs: &ort::session::SessionOutputs<'_>,
    name: &str,
    expected_trailing: usize,
) -> Result<Vec<Vec<f32>>, EstimatorError> {
    let value = outputs
        .get(name)
        .ok_or_else(|| EstimatorError::Ort(format!("output {name} missing")))?;
    // Extract as a raw `(shape, &[T])` pair so we do not couple to
    // `ort`'s internal tensor representation. The bundled Basic Pitch
    // ONNX always emits `[1, T, trailing]`-shaped tensors so we drop
    // the batch axis and reshape into a `Vec<Vec<f32>>` keyed by frame
    // index.
    let (shape, data) = value
        .try_extract_tensor::<f32>()
        .map_err(|e| EstimatorError::Ort(format!("extract {name}: {e}")))?;
    let dims: &[i64] = shape;
    if dims.len() != 3 || dims[0] != 1 || dims[2] as usize != expected_trailing {
        return Err(EstimatorError::Ort(format!(
            "{name}: unexpected shape {dims:?} (want [1, T, {expected_trailing}])"
        )));
    }
    let n_frames = dims[1] as usize;
    if data.len() != n_frames * expected_trailing {
        return Err(EstimatorError::Ort(format!(
            "{name}: tensor data length {} != {n_frames} * {expected_trailing}",
            data.len()
        )));
    }
    let mut rows: Vec<Vec<f32>> = Vec::with_capacity(n_frames);
    for t in 0..n_frames {
        let off = t * expected_trailing;
        rows.push(data[off..off + expected_trailing].to_vec());
    }
    Ok(rows)
}

/// Pack a `Vec<Vec<f32>>` whose inner rows have width 88 into a
/// `Vec<[f32; 88]>` so frame-level access is `O(1)` array indexing
/// without a bounds check on every column.
fn pack_88(rows: &[Vec<f32>]) -> Result<Vec<[f32; N_PITCH_BINS]>, EstimatorError> {
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        if row.len() != N_PITCH_BINS {
            return Err(EstimatorError::Ort(format!(
                "row width {} != expected {N_PITCH_BINS}",
                row.len()
            )));
        }
        let mut a = [0.0_f32; N_PITCH_BINS];
        a.copy_from_slice(&row[..N_PITCH_BINS]);
        out.push(a);
    }
    Ok(out)
}

/// Stitch a list of per-window outputs into one continuous output stream
/// by trimming `TRIM_FRAMES` from each interior side and concatenating
/// the surviving frames.
fn stitch_windows(windows: Vec<WindowOutput>) -> WindowOutput {
    if windows.is_empty() {
        return WindowOutput {
            onset: Vec::new(),
            note: Vec::new(),
            contour: Vec::new(),
        };
    }
    let n_windows = windows.len();
    let mut onset_out: Vec<[f32; N_PITCH_BINS]> = Vec::new();
    let mut note_out: Vec<[f32; N_PITCH_BINS]> = Vec::new();
    let mut contour_out: Vec<Vec<f32>> = Vec::new();
    for (i, w) in windows.into_iter().enumerate() {
        let start = if i == 0 { 0 } else { TRIM_FRAMES };
        let end = if i == n_windows - 1 {
            w.onset.len()
        } else {
            w.onset.len().saturating_sub(TRIM_FRAMES)
        };
        if end > start {
            onset_out.extend_from_slice(&w.onset[start..end]);
            note_out.extend_from_slice(&w.note[start..end]);
            contour_out.extend_from_slice(&w.contour[start..end]);
        }
    }
    WindowOutput {
        onset: onset_out,
        note: note_out,
        contour: contour_out,
    }
}

/// Run a per-bin Viterbi pass over the given posterior column, returning
/// a boolean activation mask. The state space is `{inactive, active}`;
/// emissions are `[1 - p, p]` so the decoder favours the more probable
/// state per frame, with the diagonal self-loop bonus killing
/// single-frame activations.
fn viterbi_activation(post: &[f32]) -> Vec<bool> {
    if post.is_empty() {
        return Vec::new();
    }
    let emissions: Vec<Vec<f32>> = post
        .iter()
        .map(|&p| {
            let p = p.clamp(1e-6, 1.0 - 1e-6);
            vec![(1.0 - p).ln(), p.ln()]
        })
        .collect();
    let model = TransitionModel {
        sigma_bins: 1.5,
        self_loop_log_bonus: 1.5,
    };
    decode(&emissions, &model)
        .into_iter()
        .map(|s| s == 1)
        .collect()
}

/// Heuristic note assembly: walk every pitch bin, open candidates on
/// onset peaks or sustained-note segments without a strong attack, and
/// close them on sustained sub-threshold frames. Mirrors Spotify's
/// `note_creation.py`: onsets drive note starts when present, but
/// sustained activation that never crosses the onset threshold (e.g.
/// from a pure synthetic tone with no transient) still opens a note
/// at the first frame the sustain head exceeds `frame_threshold`.
fn assemble_notes(
    onset: &[[f32; N_PITCH_BINS]],
    note: &[[f32; N_PITCH_BINS]],
    contour: &[Vec<f32>],
    cfg: &NoteAssemblyConfig,
    frame_rate_hz: f32,
) -> Vec<NoteEvent> {
    let n_frames = onset.len();
    if n_frames == 0 {
        return Vec::new();
    }
    let frame_ms = 1000.0 / frame_rate_hz;

    let mut events: Vec<NoteEvent> = Vec::new();
    for bin in 0..N_PITCH_BINS {
        // Pre-compute Viterbi-cleaned activation for this bin so the
        // single-frame spurious activations the raw model emits do not
        // become single-frame notes.
        let bin_post: Vec<f32> = note.iter().map(|row| row[bin]).collect();
        let active = viterbi_activation(&bin_post);

        let mut frame: usize = 0;
        while frame < n_frames {
            let onset_v = onset[frame][bin];
            // Open a note when *either* the onset crosses its
            // threshold (preferred path) *or* the Viterbi-cleaned
            // sustain head says the bin is active and the
            // raw note posterior is solidly above threshold (fallback
            // for soft attacks / pure tones).
            let open_via_onset = onset_v >= cfg.onset_threshold;
            let open_via_sustain = !open_via_onset
                && active[frame]
                && note[frame][bin] >= cfg.frame_threshold
                && (frame == 0 || !active[frame - 1]);
            if !(open_via_onset || open_via_sustain) {
                frame += 1;
                continue;
            }
            // Capture onset peak for velocity. Walk forward up to a few
            // frames in case the maximum is one or two frames after the
            // first crossing. Use the note posterior as a fallback when
            // the onset stream stayed below threshold (e.g. for soft
            // attacks); MIDI velocity should still scale with confidence.
            let mut onset_peak = onset_v.max(note[frame][bin]);
            for look in 1..=3 {
                if frame + look >= n_frames {
                    break;
                }
                let v = onset[frame + look][bin].max(note[frame + look][bin]);
                if v > onset_peak {
                    onset_peak = v;
                }
            }

            // Walk forward until the note posterior drops below
            // `frame_threshold` for `max_silent_frames + 1` consecutive
            // frames. The Viterbi-cleaned `active` mask is consulted as
            // a secondary signal: if neither the threshold nor the
            // Viterbi decision keeps the note alive, we treat the frame
            // as silent. Allowing the threshold-only path keeps notes
            // alive through the tail of the model's note posterior even
            // when the Viterbi state has already toggled to "inactive"
            // (the tail of a note hovers around 0.3-0.5 for a few
            // frames; the global Viterbi balances those against the
            // clearly silent frames that follow and switches early).
            let start_frame = frame;
            let mut end_frame = frame;
            let mut silent_run = 0;
            // The onset head emits a broad activation that can stretch
            // 2-4 frames after the true note start. Only treat a fresh
            // onset crossing as a "new note" once the onset stream has
            // *also* dropped well below the threshold for at least a
            // few frames in between — otherwise we would split every
            // attack into a series of single-frame notes.
            let mut onset_below_streak = 0_usize;
            let onset_reset_threshold: f32 = cfg.onset_threshold * 0.5;
            let mut f = frame + 1;
            while f < n_frames {
                let kept = note[f][bin] >= cfg.frame_threshold || active[f];
                if kept {
                    end_frame = f;
                    silent_run = 0;
                } else {
                    silent_run += 1;
                    if silent_run > cfg.max_silent_frames {
                        break;
                    }
                }
                if onset[f][bin] < onset_reset_threshold {
                    onset_below_streak += 1;
                } else if onset_below_streak >= ONSET_RESET_FRAMES
                    && onset[f][bin] >= cfg.onset_threshold
                {
                    // A second, well-separated attack on the same bin —
                    // close this note so the next outer-loop iteration
                    // can open a fresh one.
                    break;
                } else {
                    onset_below_streak = 0;
                }
                f += 1;
            }
            let duration = end_frame.saturating_sub(start_frame) + 1;
            if duration >= cfg.min_note_frames {
                let start_ms = (start_frame as f32 * frame_ms).round() as u64;
                let end_ms = ((end_frame + 1) as f32 * frame_ms).round() as u64;
                let velocity = (127.0 * onset_peak.clamp(0.0, 1.0))
                    .round()
                    .clamp(1.0, 127.0) as u8;
                let pitch_bend_curve = sample_pitch_bend(contour, bin, start_frame, end_frame + 1);
                events.push(NoteEvent {
                    midi: MIDI_OFFSET + bin as u8,
                    start_ms,
                    end_ms,
                    velocity,
                    pitch_bend_curve: Some(pitch_bend_curve),
                });
            }
            frame = (end_frame + 1).max(frame + 1);
        }
    }
    events
}

/// Sample the contour head's three-bin neighbourhood centred on the
/// note's MIDI bin and return one signed-cents offset per frame in
/// `[start_frame, end_frame)`. Uses a centroid weighted by the contour
/// activations; saturates at ±100 cents (the full inter-semitone
/// distance) so a single mistracked frame cannot blow the value into a
/// neighbouring note.
fn sample_pitch_bend(
    contour: &[Vec<f32>],
    bin: usize,
    start_frame: usize,
    end_frame: usize,
) -> Vec<i16> {
    let mut out = Vec::with_capacity(end_frame.saturating_sub(start_frame));
    if end_frame <= start_frame || contour.is_empty() {
        return out;
    }
    // Each MIDI bin maps to 3 contour bins centred on `bin * 3 + 1`.
    let centre_contour_bin = bin * 3 + 1;
    for f in start_frame..end_frame {
        if f >= contour.len() {
            break;
        }
        let row = &contour[f];
        if row.len() != N_CONTOUR_BINS {
            out.push(0);
            continue;
        }
        let lo = centre_contour_bin.saturating_sub(1);
        let mid = centre_contour_bin;
        let hi = (centre_contour_bin + 1).min(N_CONTOUR_BINS - 1);
        let w_lo = row[lo].max(0.0);
        let w_mid = row[mid].max(0.0);
        let w_hi = row[hi].max(0.0);
        let total = w_lo + w_mid + w_hi;
        if total <= f32::EPSILON {
            out.push(0);
            continue;
        }
        // Centroid offset in semitone-thirds (each contour bin is 1/3
        // semitone wide). Convert to cents (1 semitone = 100 cents).
        let centroid = (w_hi - w_lo) / total;
        let cents = centroid * (100.0 / 3.0);
        let clamped = cents.clamp(-100.0, 100.0).round() as i16;
        out.push(clamped);
    }
    out
}

/// Window the resampled mono buffer into chunks of size
/// `AUDIO_N_SAMPLES`, hopping by `WINDOW_HOP_SAMPLES`. Right-pads the
/// final chunk with zeros if the buffer length is not an exact multiple.
/// Each window is held as a heap-allocated `Vec<f32>` of fixed length
/// `AUDIO_N_SAMPLES` (~170 KB) to keep the stack frame small.
fn window_audio(buffer: &[f32]) -> Vec<Vec<f32>> {
    let mut windows: Vec<Vec<f32>> = Vec::new();
    if buffer.is_empty() {
        return windows;
    }
    let mut start: usize = 0;
    while start < buffer.len() {
        let mut win = vec![0.0_f32; AUDIO_N_SAMPLES];
        let end = (start + AUDIO_N_SAMPLES).min(buffer.len());
        win[..(end - start)].copy_from_slice(&buffer[start..end]);
        windows.push(win);
        if end == buffer.len() {
            break;
        }
        start += WINDOW_HOP_SAMPLES;
    }
    windows
}

impl PolyEstimator for BasicPitchEstimator {
    // The trait signature returns `&str` (with the implicit `&self`
    // lifetime). Clippy's `unnecessary_literal_bound` would prefer
    // `&'static str` here, but the trait we are implementing pins the
    // shorter lifetime — overriding to `'static` is not possible.
    #[allow(clippy::unnecessary_literal_bound)]
    fn name(&self) -> &str {
        "basic-pitch-v1"
    }

    fn analyze(
        &mut self,
        audio: &[f32],
        sample_rate_hz: u32,
    ) -> Result<PolyResult, EstimatorError> {
        let frame_rate_hz = BASIC_PITCH_SR_HZ as f32 / FFT_HOP as f32;
        let duration_ms = if sample_rate_hz == 0 {
            0
        } else {
            (audio.len() as u64 * 1000) / u64::from(sample_rate_hz)
        };
        let model_version = "basic-pitch-1.0".to_string();

        // Fast path for empty / very short input — skip ORT entirely.
        if audio.is_empty() {
            return Ok(PolyResult {
                notes: Vec::new(),
                frame_rate_hz,
                model_version,
                duration_ms,
            });
        }

        let resampled = resample_to_basic_pitch(audio, sample_rate_hz)?;
        // For inputs shorter than one full inference window we still
        // pad and run a single window so silence-only buffers exercise
        // the same code path as longer inputs.
        let buffer = if resampled.len() < AUDIO_N_SAMPLES {
            let mut padded = resampled.clone();
            padded.resize(AUDIO_N_SAMPLES, 0.0);
            padded
        } else {
            resampled
        };

        let windows = window_audio(&buffer);
        let mut window_outputs: Vec<WindowOutput> = Vec::with_capacity(windows.len());
        for window in &windows {
            let out = run_window(self, window)?;
            // Validate the output frame count matches our constants —
            // a stray re-export would be a hard failure here.
            if out.onset.len() != FRAMES_PER_WINDOW
                || out.note.len() != FRAMES_PER_WINDOW
                || out.contour.len() != FRAMES_PER_WINDOW
            {
                return Err(EstimatorError::Ort(format!(
                    "basic-pitch onnx returned unexpected frame counts \
                     (onset={}, note={}, contour={}; want {FRAMES_PER_WINDOW} each)",
                    out.onset.len(),
                    out.note.len(),
                    out.contour.len(),
                )));
            }
            window_outputs.push(out);
        }

        let stitched = stitch_windows(window_outputs);
        let notes = assemble_notes(
            &stitched.onset,
            &stitched.note,
            &stitched.contour,
            &self.assembly,
            frame_rate_hz,
        );

        // Trim notes whose timestamps run past the actual input duration
        // (which can happen because the last window is zero-padded to
        // AUDIO_N_SAMPLES).
        let notes: Vec<NoteEvent> = notes
            .into_iter()
            .filter(|n| n.start_ms < duration_ms.saturating_add(50))
            .map(|mut n| {
                if n.end_ms > duration_ms {
                    n.end_ms = duration_ms;
                }
                n
            })
            .filter(|n| n.end_ms > n.start_ms)
            .collect();

        Ok(PolyResult {
            notes,
            frame_rate_hz,
            model_version,
            duration_ms,
        })
    }
}
