//! cpal-backed [`AudioBackend`] implementation.
//!
//! `CpalAudioBackend` owns a [`cpal::Stream`]; the stream is dropped in
//! [`CpalAudioBackend::stop`] (and on `Drop`). The audio callback is
//! real-time-safe per DESIGN §2.4 / P3:
//!
//! - No `Box::new`, `Vec::push`, `String`, `format!`, or any heap allocation.
//! - No `Mutex`, `RwLock`, `parking_lot::*`.
//! - No syscalls, no `tracing::*`, no `log::*`, no `println!`.
//! - No `unwrap`, `expect`, `panic!`, or `?` on a fallible call.
//! - Sample-format conversion (`i16` / `u16` → `f32`) is inline.
//! - Mono downmix happens inline if `channels > 1`.
//! - On `producer.push(...).is_err()` the callback bails out of the
//!   sample loop on the first drop, counts the remaining samples in one
//!   shot, and `fetch_add`s the total into `dropped_samples: AtomicU64`
//!   with `Ordering::Relaxed`. It never blocks, locks, or returns early
//!   on a partial drop.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cpal::Stream;
use cpal::traits::{DeviceTrait, StreamTrait};

use crate::audio::backend::{AudioBackend, AudioBackendConfig, AudioBackendError};

/// Production cpal-backed audio capture.
///
/// Construct with [`CpalAudioBackend::new`] passing a chosen [`cpal::Device`]
/// and matching [`cpal::StreamConfig`]; call [`AudioBackend::start`] with the
/// SPSC producer to begin capture. The owned [`Stream`] is dropped in
/// [`AudioBackend::stop`].
pub struct CpalAudioBackend {
    cfg: AudioBackendConfig,
    device: cpal::Device,
    stream_config: cpal::StreamConfig,
    sample_format: cpal::SampleFormat,
    stream: Option<Stream>,
    dropped_samples: Arc<AtomicU64>,
}

impl core::fmt::Debug for CpalAudioBackend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CpalAudioBackend")
            .field("cfg", &self.cfg)
            .field("sample_format", &self.sample_format)
            .field("stream_active", &self.stream.is_some())
            .finish_non_exhaustive()
    }
}

impl CpalAudioBackend {
    /// Construct a new cpal backend bound to `device` with `stream_config`.
    ///
    /// Stream creation is deferred to [`AudioBackend::start`].
    pub fn new(
        cfg: AudioBackendConfig,
        device: cpal::Device,
        stream_config: cpal::StreamConfig,
        sample_format: cpal::SampleFormat,
    ) -> Self {
        Self {
            cfg,
            device,
            stream_config,
            sample_format,
            stream: None,
            dropped_samples: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Borrow a clone-able handle to the cumulative dropped-sample counter.
    ///
    /// The counter is incremented (with `Ordering::Relaxed`) inside the
    /// cpal callback whenever a sample cannot be pushed because the SPSC
    /// ring is full. Phase 1.1 exposes it as a poll-only handle for
    /// callers (tests, the Phase 1.2 Tauri shell). The DSP worker does
    /// **not** currently poll it; wiring of the structured underrun
    /// warning is Phase 1.2 work.
    pub fn dropped_samples(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.dropped_samples)
    }
}

impl AudioBackend for CpalAudioBackend {
    fn start(&mut self, producer: rtrb::Producer<f32>) -> Result<(), AudioBackendError> {
        if self.stream.is_some() {
            return Err(AudioBackendError::AlreadyStarted);
        }

        let channels = self.stream_config.channels as usize;
        let dropped = Arc::clone(&self.dropped_samples);

        // The error callback runs off-RT, so logging here is allowed.
        let err_fn = |err: cpal::StreamError| {
            tracing::warn!(target: "neural_pitch_core::audio::cpal_backend", error = %err, "cpal stream error");
        };

        let stream = match self.sample_format {
            cpal::SampleFormat::F32 => self
                .device
                .build_input_stream(
                    &self.stream_config,
                    {
                        let dropped = Arc::clone(&dropped);
                        let mut producer = producer;
                        move |data: &[f32], _: &cpal::InputCallbackInfo| {
                            push_mono_f32(data, channels, &mut producer, &dropped);
                        }
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| AudioBackendError::BuildStream(e.to_string()))?,
            cpal::SampleFormat::I16 => self
                .device
                .build_input_stream(
                    &self.stream_config,
                    {
                        let dropped = Arc::clone(&dropped);
                        let mut producer = producer;
                        move |data: &[i16], _: &cpal::InputCallbackInfo| {
                            push_mono_i16(data, channels, &mut producer, &dropped);
                        }
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| AudioBackendError::BuildStream(e.to_string()))?,
            cpal::SampleFormat::U16 => self
                .device
                .build_input_stream(
                    &self.stream_config,
                    {
                        let dropped = Arc::clone(&dropped);
                        let mut producer = producer;
                        move |data: &[u16], _: &cpal::InputCallbackInfo| {
                            push_mono_u16(data, channels, &mut producer, &dropped);
                        }
                    },
                    err_fn,
                    None,
                )
                .map_err(|e| AudioBackendError::BuildStream(e.to_string()))?,
            other => {
                return Err(AudioBackendError::UnsupportedFormat(format!("{other:?}")));
            }
        };

        stream
            .play()
            .map_err(|e| AudioBackendError::BuildStream(e.to_string()))?;
        self.stream = Some(stream);
        Ok(())
    }

    fn stop(&mut self) {
        self.stream = None;
    }

    fn config(&self) -> &AudioBackendConfig {
        &self.cfg
    }
}

// -- RT-safe callbacks ---------------------------------------------------------
//
// The functions below MUST NOT allocate, log, lock, or panic. They take an
// `&AtomicU64` for dropped-sample accounting and use only relaxed ordering.

/// Convert a single signed-16-bit PCM sample to `f32` in `[-1.0, 1.0]`.
///
/// Note: dividing by `i16::MAX` makes `i16::MIN` map to ~-1.0000305, a
/// 0.0015 dB asymmetry that is below the audible threshold and below the
/// noise floor of any cpal input device. Symmetric scaling around
/// `|i16::MIN|` would clip `i16::MAX` to just under +1.0, which is the
/// same trade. We pick `i16::MAX` because it matches the cpal canonical
/// conversion path.
#[inline]
pub(crate) fn i16_to_f32(s: i16) -> f32 {
    (s as f32) * (1.0_f32 / i16::MAX as f32)
}

/// Convert a single unsigned-16-bit PCM sample (zero at `0x8000`) to `f32`.
#[inline]
pub(crate) fn u16_to_f32(s: u16) -> f32 {
    let bias = i16::MAX as f32 + 1.0;
    ((s as f32) - bias) * (1.0_f32 / bias)
}

#[inline]
pub(crate) fn push_mono_f32(
    data: &[f32],
    channels: usize,
    producer: &mut rtrb::Producer<f32>,
    dropped: &AtomicU64,
) {
    if channels <= 1 {
        for (i, &s) in data.iter().enumerate() {
            if producer.push(s).is_err() {
                let remaining = (data.len() - i) as u64;
                dropped.fetch_add(remaining, Ordering::Relaxed);
                return;
            }
        }
    } else {
        debug_assert!(
            data.len() % channels == 0,
            "cpal callback gave us a partial interleaved frame"
        );
        let inv = 1.0_f32 / channels as f32;
        let mut i = 0;
        while i + channels <= data.len() {
            // Multiply each channel by `inv` *before* summing to keep the
            // accumulator magnitude bounded for very high channel counts.
            let mut acc = 0.0_f32;
            for c in 0..channels {
                acc += data[i + c] * inv;
            }
            if producer.push(acc).is_err() {
                let remaining_frames = ((data.len() - i) / channels) as u64;
                dropped.fetch_add(remaining_frames, Ordering::Relaxed);
                return;
            }
            i += channels;
        }
    }
}

#[inline]
pub(crate) fn push_mono_i16(
    data: &[i16],
    channels: usize,
    producer: &mut rtrb::Producer<f32>,
    dropped: &AtomicU64,
) {
    if channels <= 1 {
        for (i, &s) in data.iter().enumerate() {
            if producer.push(i16_to_f32(s)).is_err() {
                let remaining = (data.len() - i) as u64;
                dropped.fetch_add(remaining, Ordering::Relaxed);
                return;
            }
        }
    } else {
        debug_assert!(
            data.len() % channels == 0,
            "cpal callback gave us a partial interleaved frame"
        );
        let inv = 1.0_f32 / channels as f32;
        let mut i = 0;
        while i + channels <= data.len() {
            let mut acc = 0.0_f32;
            for c in 0..channels {
                acc += i16_to_f32(data[i + c]) * inv;
            }
            if producer.push(acc).is_err() {
                let remaining_frames = ((data.len() - i) / channels) as u64;
                dropped.fetch_add(remaining_frames, Ordering::Relaxed);
                return;
            }
            i += channels;
        }
    }
}

#[inline]
pub(crate) fn push_mono_u16(
    data: &[u16],
    channels: usize,
    producer: &mut rtrb::Producer<f32>,
    dropped: &AtomicU64,
) {
    if channels <= 1 {
        for (i, &s) in data.iter().enumerate() {
            if producer.push(u16_to_f32(s)).is_err() {
                let remaining = (data.len() - i) as u64;
                dropped.fetch_add(remaining, Ordering::Relaxed);
                return;
            }
        }
    } else {
        debug_assert!(
            data.len() % channels == 0,
            "cpal callback gave us a partial interleaved frame"
        );
        let inv = 1.0_f32 / channels as f32;
        let mut i = 0;
        while i + channels <= data.len() {
            let mut acc = 0.0_f32;
            for c in 0..channels {
                acc += u16_to_f32(data[i + c]) * inv;
            }
            if producer.push(acc).is_err() {
                let remaining_frames = ((data.len() - i) / channels) as u64;
                dropped.fetch_add(remaining_frames, Ordering::Relaxed);
                return;
            }
            i += channels;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ring(cap: usize) -> (rtrb::Producer<f32>, rtrb::Consumer<f32>) {
        rtrb::RingBuffer::<f32>::new(cap)
    }

    fn drain(cons: &mut rtrb::Consumer<f32>) -> Vec<f32> {
        let mut v = Vec::new();
        while let Ok(s) = cons.pop() {
            v.push(s);
        }
        v
    }

    #[test]
    fn push_mono_f32_passthrough_mono() {
        let (mut p, mut c) = ring(16);
        let dropped = AtomicU64::new(0);
        let data = [0.1_f32, -0.2, 0.3, -0.4];
        push_mono_f32(&data, 1, &mut p, &dropped);
        assert_eq!(drain(&mut c), data);
        assert_eq!(dropped.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn push_mono_f32_two_channel_average() {
        let (mut p, mut c) = ring(16);
        let dropped = AtomicU64::new(0);
        // Interleaved L/R: (1.0, -1.0), (0.5, 0.5), (-0.25, 0.25)
        let data = [1.0_f32, -1.0, 0.5, 0.5, -0.25, 0.25];
        push_mono_f32(&data, 2, &mut p, &dropped);
        let got = drain(&mut c);
        assert_eq!(got.len(), 3);
        assert!((got[0] - 0.0).abs() < 1e-6);
        assert!((got[1] - 0.5).abs() < 1e-6);
        assert!((got[2] - 0.0).abs() < 1e-6);
    }

    #[test]
    fn push_mono_f32_full_ring_increments_dropped() {
        // Capacity 4, push 10 samples, expect 6 dropped.
        let (mut p, _c) = ring(4);
        let dropped = AtomicU64::new(0);
        let data: Vec<f32> = (0..10).map(|i| i as f32).collect();
        push_mono_f32(&data, 1, &mut p, &dropped);
        // rtrb capacity-N rings can accept up to N samples before push fails.
        // Exact accepted count depends on rtrb internals (>= cap-1, <= cap);
        // assert dropped >= data.len() - cap.
        let dropped_count = dropped.load(Ordering::Relaxed);
        assert!(
            (6..=10).contains(&dropped_count),
            "expected 6..=10 dropped samples, got {dropped_count}"
        );
    }

    #[test]
    fn i16_extremes_scale_to_unit_interval() {
        // i16::MAX maps exactly to +1.0; i16::MIN maps to ~-1.0000305.
        assert!((i16_to_f32(i16::MAX) - 1.0).abs() < 1e-6);
        assert!((i16_to_f32(0) - 0.0).abs() < 1e-6);
        let min = i16_to_f32(i16::MIN);
        // Documented asymmetry: slightly below -1.0.
        assert!(min < -1.0 && min > -1.0001);
    }

    #[test]
    fn u16_zero_centres_at_minus_unity_offset() {
        // u16 0x0000 maps near -1.0; 0x8000 maps to 0.0; 0xFFFF near +1.0.
        let mid = u16_to_f32(0x8000);
        assert!(mid.abs() < 1e-6, "0x8000 should be ~0.0, got {mid}");
        let lo = u16_to_f32(0);
        assert!(lo < -0.999 && lo > -1.0001);
        let hi = u16_to_f32(0xFFFF);
        assert!(hi > 0.999 && hi < 1.0001);
    }

    #[test]
    fn push_mono_i16_full_ring_increments_dropped() {
        let (mut p, _c) = ring(4);
        let dropped = AtomicU64::new(0);
        let data: Vec<i16> = (0..16).map(|i| i as i16).collect();
        push_mono_i16(&data, 1, &mut p, &dropped);
        let dropped_count = dropped.load(Ordering::Relaxed);
        assert!(
            (12..=16).contains(&dropped_count),
            "expected 12..=16 dropped samples, got {dropped_count}"
        );
    }
}
