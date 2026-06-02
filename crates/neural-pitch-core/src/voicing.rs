//! Voice-activity gating for monophonic pitch pipelines.
//!
//! [`VoiceActivityGate`] reports `true` when the RMS energy of a sample
//! chunk exceeds a configured threshold, with a hangover period that keeps
//! the gate open for `hangover_frames` after the last above-threshold chunk.
//! This avoids choppy on/off cycling at the boundary of a sustained note.

/// RMS-threshold voice activity gate with hangover.
///
/// The gate is stateful: callers feed it chunks of samples in arrival order
/// and read back a boolean. It is intended to be used as a caller-side
/// supplement to a pitch estimator's internal voicing decision — see
/// [`crate::pitch::F0Frame::voiced`].
#[derive(Debug, Clone)]
pub struct VoiceActivityGate {
    /// RMS threshold, on the same scale as the input samples (typically
    /// `[-1.0, 1.0]`). Samples with RMS strictly greater than this are
    /// considered voiced.
    pub rms_threshold: f32,

    /// Number of frames the gate stays open after the most recent
    /// above-threshold chunk.
    pub hangover_frames: u32,

    /// Frames elapsed since the last above-threshold chunk. Public so
    /// pipelines can introspect for diagnostics.
    pub frames_since_above: u32,
}

impl VoiceActivityGate {
    /// Construct a new gate with the given threshold and hangover length.
    pub fn new(rms_threshold: f32, hangover_frames: u32) -> Self {
        Self {
            rms_threshold,
            hangover_frames,
            // Start the counter saturated so a fresh gate reports unvoiced
            // until the first above-threshold chunk arrives.
            frames_since_above: hangover_frames.saturating_add(1),
        }
    }

    /// Process one chunk of samples and return the gate state.
    ///
    /// Empty chunks are treated as silence: they advance the hangover
    /// counter without resetting it.
    pub fn is_voiced(&mut self, samples: &[f32]) -> bool {
        let rms = if samples.is_empty() {
            0.0_f32
        } else {
            let sum_sq: f32 = samples.iter().map(|s| s * s).sum();
            (sum_sq / samples.len() as f32).sqrt()
        };
        if rms > self.rms_threshold {
            self.frames_since_above = 0;
            true
        } else {
            self.frames_since_above = self.frames_since_above.saturating_add(1);
            self.frames_since_above <= self.hangover_frames
        }
    }

    /// Drop the hangover counter, returning the gate to its just-constructed
    /// state.
    pub fn reset(&mut self) {
        self.frames_since_above = self.hangover_frames.saturating_add(1);
    }
}
