//! HTDemucs ONNX session wrapper and per-segment forward pass.
//!
//! Holds an `ort::Session` and exposes a single-segment
//! `forward(stereo_44k1: [1, 2, 352_800]) -> [1, 4, 2, 352_800]` call.
//! The I/O signature is resolved defensively at construction time so
//! exports that emit four named outputs (one per stem) instead of one
//! stacked tensor still resolve cleanly — same defensive name-suffix
//! tiebreak pattern as `poly::basic_pitch::from_bytes`.

#![cfg(feature = "neural")]

use std::path::Path;

use tracing::instrument;

use crate::stems::StemError;
use crate::stems::segment::SEGMENT_SAMPLES;

/// Number of stems HTDemucs emits per forward pass.
///
/// Always four (vocals, drums, bass, other) for the canonical export.
pub const N_STEMS: usize = 4;

/// Stem ordering used inside [`SegmentOutput`] when the model emits a
/// single stacked `[1, 4, 2, T]` output tensor: vocals, drums, bass,
/// other (matches the upstream Demucs reference order).
const STEM_ORDER: [&str; N_STEMS] = ["vocals", "drums", "bass", "other"];

/// Per-segment output: four interleaved stereo `[2, T]` buffers.
///
/// Held flat as four separate `Vec<f32>` so the segment-overlap-add
/// stage can mix straight into the four output stems without a
/// transpose.
#[derive(Debug)]
#[must_use]
pub struct SegmentOutput {
    /// Vocals buffer, layout `[ch0, ch1, ch0, ch1, ...]` interleaved.
    pub vocals: Vec<f32>,
    /// Drums buffer, layout matches `vocals`.
    pub drums: Vec<f32>,
    /// Bass buffer, layout matches `vocals`.
    pub bass: Vec<f32>,
    /// Other buffer, layout matches `vocals`.
    pub other: Vec<f32>,
}

/// Resolved output naming for the four stems. Either:
///
/// * `Stacked { name }` — model emits a single 4-D tensor of shape
///   `[1, 4, 2, T]` named `name`. We slice the leading axis at
///   inference time.
/// * `PerStem { vocals, drums, bass, other }` — model emits four
///   3-D tensors of shape `[1, 2, T]` each, one per stem.
#[derive(Debug)]
enum OutputBinding {
    Stacked {
        name: String,
    },
    PerStem {
        vocals: String,
        drums: String,
        bass: String,
        other: String,
    },
}

/// Opaque ONNX session holder.
///
/// Hidden behind a struct so the `ort` crate does not leak through
/// the public API surface.
#[allow(clippy::struct_field_names)]
pub struct Session {
    session: Box<ort::session::Session>,
    input_name: String,
    output_binding: OutputBinding,
    /// Reusable de-interleave scratch buffer. Per-segment forward calls
    /// rewrite the planar layout into this buffer instead of
    /// allocating a fresh ~2.8 MB block per segment, so a 4-min track
    /// (~30 segments) saves ~85 MB of allocator churn over the run.
    planar: Vec<f32>,
}

impl Session {
    /// Open the HTDemucs ONNX file.
    pub fn open(model_path: &Path) -> Result<Self, StemError> {
        if !model_path.exists() {
            return Err(StemError::ModelNotFound(model_path.to_path_buf()));
        }
        let bytes = std::fs::read(model_path)?;
        let session = ort::session::Session::builder()
            .map_err(|e| StemError::Ort(format!("session builder: {e}")))?
            .commit_from_memory(&bytes)
            .map_err(|e| StemError::Ort(format!("commit_from_memory: {e}")))?;

        let input_name = session
            .inputs()
            .first()
            .map(|i| i.name().to_string())
            .ok_or_else(|| StemError::Ort("htdemucs onnx has no inputs".to_string()))?;

        let output_binding = resolve_output_binding(&session)?;

        Ok(Self {
            session: Box::new(session),
            input_name,
            output_binding,
            planar: vec![0.0_f32; 2 * SEGMENT_SAMPLES],
        })
    }

    /// Run a single forward pass on one 8-second stereo segment.
    ///
    /// `stereo_segment` MUST be exactly
    /// `2 * crate::stems::segment::SEGMENT_SAMPLES` floats long
    /// (interleaved stereo at 44.1 kHz).
    #[instrument(skip_all)]
    pub fn forward(&mut self, stereo_segment: &[f32]) -> Result<SegmentOutput, StemError> {
        let expected = 2 * SEGMENT_SAMPLES;
        if stereo_segment.len() != expected {
            return Err(StemError::Configuration(format!(
                "htdemucs forward expects {expected} interleaved stereo samples; got {}",
                stereo_segment.len()
            )));
        }

        // De-interleave into the [1, 2, T] planar layout HTDemucs
        // expects. The scratch buffer is held inside `self` and reused
        // across segments to avoid per-call ~2.8 MB allocator churn.
        debug_assert_eq!(self.planar.len(), 2 * SEGMENT_SAMPLES);
        let (left, right) = self.planar.split_at_mut(SEGMENT_SAMPLES);
        for t in 0..SEGMENT_SAMPLES {
            left[t] = stereo_segment[2 * t];
            right[t] = stereo_segment[2 * t + 1];
        }
        let shape: [usize; 3] = [1, 2, SEGMENT_SAMPLES];
        let input_value = ort::value::TensorRef::from_array_view((shape, self.planar.as_slice()))
            .map_err(|e| StemError::Ort(format!("tensor view: {e}")))?;
        let inputs: Vec<(
            std::borrow::Cow<'_, str>,
            ort::session::SessionInputValue<'_>,
        )> = vec![(
            std::borrow::Cow::Borrowed(self.input_name.as_str()),
            ort::session::SessionInputValue::from(input_value),
        )];

        let outputs = self
            .session
            .run(inputs)
            .map_err(|e| StemError::Ort(format!("session run: {e}")))?;

        match &self.output_binding {
            OutputBinding::Stacked { name } => extract_stacked(&outputs, name),
            OutputBinding::PerStem {
                vocals,
                drums,
                bass,
                other,
            } => Ok(SegmentOutput {
                vocals: extract_per_stem(&outputs, vocals)?,
                drums: extract_per_stem(&outputs, drums)?,
                bass: extract_per_stem(&outputs, bass)?,
                other: extract_per_stem(&outputs, other)?,
            }),
        }
    }
}

/// Inspect the session's output metadata and decide whether the model
/// emits one stacked `[1, 4, 2, T]` tensor or four separate `[1, 2, T]`
/// tensors. Either layout is supported in the wild.
fn resolve_output_binding(session: &ort::session::Session) -> Result<OutputBinding, StemError> {
    let outputs = session.outputs();
    if outputs.is_empty() {
        return Err(StemError::Ort("htdemucs onnx has no outputs".to_string()));
    }

    // Single-output case: assume it is the stacked `[1, 4, 2, T]`
    // tensor. `try_extract_tensor` at run-time will catch a
    // shape mismatch.
    if outputs.len() == 1 {
        return Ok(OutputBinding::Stacked {
            name: outputs[0].name().to_string(),
        });
    }

    // Four-output case: bind by name suffix when available, otherwise
    // fall back to the upstream output order.
    let mut by_suffix: [Option<String>; N_STEMS] = Default::default();
    let mut leftovers: Vec<String> = Vec::new();
    for outlet in outputs {
        let name = outlet.name().to_string();
        let lower = name.to_ascii_lowercase();
        let mut matched = false;
        for (i, key) in STEM_ORDER.iter().enumerate() {
            if lower.contains(key) && by_suffix[i].is_none() {
                by_suffix[i] = Some(name.clone());
                matched = true;
                break;
            }
        }
        if !matched {
            leftovers.push(name);
        }
    }
    // Fill any unresolved slot from the leftovers in order.
    let mut leftovers = leftovers.into_iter();
    for slot in &mut by_suffix {
        if slot.is_none()
            && let Some(next) = leftovers.next()
        {
            *slot = Some(next);
        }
    }
    let [vocals, drums, bass, other] = by_suffix;
    Ok(OutputBinding::PerStem {
        vocals: vocals.ok_or_else(|| StemError::Ort("missing vocals output".to_string()))?,
        drums: drums.ok_or_else(|| StemError::Ort("missing drums output".to_string()))?,
        bass: bass.ok_or_else(|| StemError::Ort("missing bass output".to_string()))?,
        other: other.ok_or_else(|| StemError::Ort("missing other output".to_string()))?,
    })
}

/// Extract a `[1, 4, 2, T]` stacked output and split into four
/// interleaved-stereo buffers.
fn extract_stacked(
    outputs: &ort::session::SessionOutputs<'_>,
    name: &str,
) -> Result<SegmentOutput, StemError> {
    let value = outputs
        .get(name)
        .ok_or_else(|| StemError::Ort(format!("output {name} missing")))?;
    let (shape, data) = value
        .try_extract_tensor::<f32>()
        .map_err(|e| StemError::Ort(format!("extract {name}: {e}")))?;
    let dims: &[i64] = shape;
    if dims.len() != 4 || dims[0] != 1 || dims[1] != 4 || dims[2] != 2 {
        return Err(StemError::Ort(format!(
            "{name}: unexpected stacked shape {dims:?} (want [1, 4, 2, T])"
        )));
    }
    let t = dims[3] as usize;
    let stem_stride = 2 * t;
    if data.len() != 4 * stem_stride {
        return Err(StemError::Ort(format!(
            "{name}: tensor data length {} != 4 * 2 * {t}",
            data.len()
        )));
    }
    let make_stem = |stem_idx: usize| -> Vec<f32> {
        let off = stem_idx * stem_stride;
        let ch0 = &data[off..off + t];
        let ch1 = &data[off + t..off + 2 * t];
        let mut interleaved = Vec::with_capacity(2 * t);
        for i in 0..t {
            interleaved.push(ch0[i]);
            interleaved.push(ch1[i]);
        }
        interleaved
    };
    Ok(SegmentOutput {
        vocals: make_stem(0),
        drums: make_stem(1),
        bass: make_stem(2),
        other: make_stem(3),
    })
}

/// Extract a `[1, 2, T]` per-stem output and re-interleave to stereo.
fn extract_per_stem(
    outputs: &ort::session::SessionOutputs<'_>,
    name: &str,
) -> Result<Vec<f32>, StemError> {
    let value = outputs
        .get(name)
        .ok_or_else(|| StemError::Ort(format!("output {name} missing")))?;
    let (shape, data) = value
        .try_extract_tensor::<f32>()
        .map_err(|e| StemError::Ort(format!("extract {name}: {e}")))?;
    let dims: &[i64] = shape;
    if dims.len() != 3 || dims[0] != 1 || dims[1] != 2 {
        return Err(StemError::Ort(format!(
            "{name}: unexpected per-stem shape {dims:?} (want [1, 2, T])"
        )));
    }
    let t = dims[2] as usize;
    if data.len() != 2 * t {
        return Err(StemError::Ort(format!(
            "{name}: tensor data length {} != 2 * {t}",
            data.len()
        )));
    }
    let ch0 = &data[..t];
    let ch1 = &data[t..2 * t];
    let mut interleaved = Vec::with_capacity(2 * t);
    for i in 0..t {
        interleaved.push(ch0[i]);
        interleaved.push(ch1[i]);
    }
    Ok(interleaved)
}
