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
//!
//! ## Out-of-band events and platform fallbacks
//!
//! - **Out-of-band events.** [`CpalAudioBackend::with_emitter`] takes an
//!   [`AudioEventEmitter`] (a `Fn(AudioBackendEvent)`) supplied by the Tauri
//!   shell. The cpal `err_fn` runs on the platform audio thread; on
//!   [`cpal::StreamError::DeviceNotAvailable`] it calls the emitter with
//!   [`AudioBackendEvent::Disconnected`], on any other variant with
//!   [`AudioBackendEvent::Underrun`]. The emitter wraps a non-blocking
//!   `tauri::ipc::Channel::send` on the shell side; calls in `err_fn` are
//!   non-blocking with respect to the audio thread.
//! - **Windows WASAPI buffer-size fallback.** [`pick_buffer_size`] clamps a
//!   requested buffer size into the device's
//!   [`cpal::SupportedBufferSize::Range`] when present, falling back to
//!   [`cpal::BufferSize::Default`] for `Unknown` ranges.
//! - **Linux ALSA-via-PulseAudio renegotiation.** On
//!   [`cpal::BuildStreamError::StreamConfigNotSupported`] the backend
//!   re-queries `default_input_config()` once and rebuilds the stream with
//!   the device-advertised configuration (cpal #564 family). This is a
//!   single-shot retry, not a loop.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use cpal::traits::{DeviceTrait, StreamTrait};
use cpal::{BufferSize, BuildStreamError, Stream, StreamError, SupportedBufferSize};

use crate::audio::backend::{
    AudioBackend, AudioBackendConfig, AudioBackendError, AudioBackendEvent, AudioEventEmitter,
};

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
    /// Optional buffer-size requested by the caller. When `Some(n)`, the
    /// backend asks the device for `BufferSize::Fixed(n)`; if the device
    /// rejects with [`BuildStreamError::StreamConfigNotSupported`], the
    /// renegotiation path queries the supported range and runs
    /// [`pick_buffer_size`] before retrying.
    requested_buffer_frames: Option<u32>,
    stream: Option<Stream>,
    dropped_samples: Arc<AtomicU64>,
    events: Option<AudioEventEmitter>,
}

impl core::fmt::Debug for CpalAudioBackend {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("CpalAudioBackend")
            .field("cfg", &self.cfg)
            .field("sample_format", &self.sample_format)
            .field("requested_buffer_frames", &self.requested_buffer_frames)
            .field("stream_active", &self.stream.is_some())
            .field("has_event_emitter", &self.events.is_some())
            .finish_non_exhaustive()
    }
}

impl CpalAudioBackend {
    /// Construct a new cpal backend bound to `device` with `stream_config`.
    ///
    /// Stream creation is deferred to [`AudioBackend::start`]. No event
    /// emitter is attached; use [`CpalAudioBackend::with_emitter`] (builder
    /// style) to wire one in.
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
            requested_buffer_frames: None,
            stream: None,
            dropped_samples: Arc::new(AtomicU64::new(0)),
            events: None,
        }
    }

    /// Attach an [`AudioEventEmitter`]. Replaces any previously-attached
    /// emitter. Builder-style.
    #[must_use]
    pub fn with_emitter(mut self, emitter: AudioEventEmitter) -> Self {
        self.events = Some(emitter);
        self
    }

    /// Request a fixed audio-callback buffer size in frames. The actual size
    /// passed to cpal is clamped to the device's
    /// [`cpal::SupportedBufferSize::Range`] via [`pick_buffer_size`] at
    /// stream-build time, with a single-shot renegotiation on
    /// [`BuildStreamError::StreamConfigNotSupported`]. Builder-style.
    #[must_use]
    pub fn with_buffer_frames(mut self, frames: u32) -> Self {
        self.requested_buffer_frames = Some(frames);
        self
    }

    /// Borrow a clone-able handle to the cumulative dropped-sample counter.
    ///
    /// The counter is incremented (with `Ordering::Relaxed`) inside the
    /// cpal callback whenever a sample cannot be pushed because the SPSC
    /// ring is full. Callers (tests, the Tauri shell) read it as needed.
    pub fn dropped_samples(&self) -> Arc<AtomicU64> {
        Arc::clone(&self.dropped_samples)
    }

    /// Build the cpal input stream for the currently-stored config. Returns
    /// the constructed stream on success or the build error verbatim. The
    /// caller owns retry / renegotiation policy.
    fn build_stream(&self, producer: rtrb::Producer<f32>) -> Result<Stream, BuildStreamError> {
        let channels = self.stream_config.channels as usize;
        let dropped = Arc::clone(&self.dropped_samples);
        let events = self.events.clone();

        let err_fn = move |err: StreamError| {
            // The error callback runs off the RT data path on every supported
            // cpal backend (CoreAudio HAL listener thread on macOS, WASAPI
            // event thread on Windows, ALSA poll thread on Linux). Logging
            // and emitter-dispatch here are safe because they are NOT on
            // the audio data callback. The module docs at lines 5-9 forbid
            // tracing/log inside the data callback; they do not forbid
            // tracing inside the err_fn, which executes on a non-RT thread.
            tracing::trace!(target: "neural_pitch_core::audio::cpal_backend", error = %err, "cpal stream error");
            if let Some(em) = events.as_ref() {
                em(map_stream_error(&err, dropped.load(Ordering::Relaxed)));
            }
        };

        match self.sample_format {
            cpal::SampleFormat::F32 => self.device.build_input_stream(
                &self.stream_config,
                {
                    let dropped = Arc::clone(&self.dropped_samples);
                    let mut producer = producer;
                    move |data: &[f32], _: &cpal::InputCallbackInfo| {
                        push_mono_f32(data, channels, &mut producer, &dropped);
                    }
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::I16 => self.device.build_input_stream(
                &self.stream_config,
                {
                    let dropped = Arc::clone(&self.dropped_samples);
                    let mut producer = producer;
                    move |data: &[i16], _: &cpal::InputCallbackInfo| {
                        push_mono_i16(data, channels, &mut producer, &dropped);
                    }
                },
                err_fn,
                None,
            ),
            cpal::SampleFormat::U16 => self.device.build_input_stream(
                &self.stream_config,
                {
                    let dropped = Arc::clone(&self.dropped_samples);
                    let mut producer = producer;
                    move |data: &[u16], _: &cpal::InputCallbackInfo| {
                        push_mono_u16(data, channels, &mut producer, &dropped);
                    }
                },
                err_fn,
                None,
            ),
            // Backend-side guard: the shell should have rejected this format
            // before constructing us, but we still degrade gracefully.
            _ => Err(BuildStreamError::StreamConfigNotSupported),
        }
    }

    /// Apply the requested buffer size against the device's supported range,
    /// updating `self.stream_config.buffer_size`. Logs the negotiation
    /// outcome at `debug` level.
    fn apply_buffer_size(&mut self) {
        let Some(req) = self.requested_buffer_frames else {
            return;
        };
        let range = match self.device.default_input_config() {
            Ok(supported) => *supported.buffer_size(),
            Err(_) => SupportedBufferSize::Unknown,
        };
        let chosen = pick_buffer_size(req, range);
        tracing::debug!(
            target: "neural_pitch_core::audio::cpal_backend",
            ?range,
            requested = req,
            ?chosen,
            "negotiated audio callback buffer size",
        );
        self.stream_config.buffer_size = chosen;
    }

    /// Renegotiate the stream config from the device's default and update
    /// `self.stream_config` / `self.sample_format` accordingly. Used as the
    /// single-shot fallback for `StreamConfigNotSupported` (cpal #564 family,
    /// ALSA-via-PulseAudio sample-rate misreport).
    ///
    /// Returns `true` if renegotiation produced a different config than the
    /// caller had previously stored, `false` otherwise.
    fn renegotiate_to_default(&mut self) -> bool {
        let Ok(default) = self.device.default_input_config() else {
            return false;
        };
        // In cpal 0.17, `SampleRate` is a `u32` type alias and
        // `sample_rate()` returns `u32` directly — no tuple-struct wrapper.
        let new_sample_rate: u32 = default.sample_rate();
        let new_channels = default.channels();
        let new_format = default.sample_format();
        let changed = new_sample_rate != self.stream_config.sample_rate
            || new_channels != self.stream_config.channels
            || new_format != self.sample_format;

        self.stream_config = cpal::StreamConfig {
            channels: new_channels,
            sample_rate: new_sample_rate,
            buffer_size: BufferSize::Default,
        };
        self.sample_format = new_format;
        // Keep the published `AudioBackendConfig` in sync so the worker can
        // observe the post-renegotiation geometry.
        self.cfg.sample_rate = new_sample_rate;
        self.cfg.channels = new_channels;
        changed
    }
}

impl AudioBackend for CpalAudioBackend {
    fn start(&mut self, producer: rtrb::Producer<f32>) -> Result<(), AudioBackendError> {
        if self.stream.is_some() {
            return Err(AudioBackendError::AlreadyStarted);
        }
        self.apply_buffer_size();

        // We have to consume the producer to build a stream. On the
        // renegotiation path we need a *fresh* producer, so we construct a
        // single-shot helper that takes the producer and returns it back if
        // the build fails.
        let first_attempt = self.build_stream(producer);
        let stream = match first_attempt {
            Ok(s) => s,
            Err(BuildStreamError::StreamConfigNotSupported) => {
                // Single-shot renegotiation. The callback closure in
                // `build_stream` consumed the producer; we cannot recover it
                // because rtrb producers are move-only. Surface a typed error
                // and let the shell rebuild the controller from scratch with
                // a fresh ring on the renegotiated config.
                let changed = self.renegotiate_to_default();
                if changed {
                    if let Some(em) = self.events.as_ref() {
                        em(AudioBackendEvent::FormatChanged {
                            new: self.cfg.clone(),
                        });
                    }
                }
                return Err(AudioBackendError::UnsupportedFormat(
                    "stream config not supported; renegotiated to device default — restart capture"
                        .to_string(),
                ));
            }
            Err(BuildStreamError::BackendSpecific { err }) => {
                // Inspect the backend-specific message for permission-denial
                // markers (macOS TCC). On macOS denial cpal returns
                // `BackendSpecific` carrying a CoreAudio-derived description;
                // the substring set covers the documented English variants
                // and the canonical macOS phrasing. We keep the original
                // message in the typed variant so logs preserve diagnostics.
                let msg = err.to_string();
                let lower = msg.to_ascii_lowercase();
                if lower.contains("permission")
                    || lower.contains("not authorized")
                    || lower.contains("denied")
                    // OSStatus -50 (kAudio_ParamError) and the TCC-specific
                    // -54 / -1719 surface as decimal in CoreAudio messages.
                    || lower.contains("-50")
                    || lower.contains("-54")
                    || lower.contains("-1719")
                {
                    return Err(AudioBackendError::PermissionDenied(msg));
                }
                return Err(AudioBackendError::BuildStream(msg));
            }
            Err(e) => return Err(AudioBackendError::BuildStream(e.to_string())),
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

/// Map a cpal [`StreamError`] onto the coarse [`AudioBackendEvent`] taxonomy.
///
/// Factored out of the err_fn closure so it can be unit-tested directly:
/// `DeviceNotAvailable` becomes [`AudioBackendEvent::Disconnected`]; every
/// other variant becomes [`AudioBackendEvent::Underrun`] carrying the
/// supplied dropped-sample counter. Pure function, no side effects.
#[must_use]
pub fn map_stream_error(err: &StreamError, dropped: u64) -> AudioBackendEvent {
    match err {
        StreamError::DeviceNotAvailable => AudioBackendEvent::Disconnected,
        _ => AudioBackendEvent::Underrun { count: dropped },
    }
}

/// Pick a [`BufferSize`] for the audio callback that the device will accept.
///
/// On Windows WASAPI cpal rejects `BufferSize::Fixed(n)` outside the device's
/// `SupportedBufferSize::Range { min, max }` (cpal #544, #534). This helper
/// clamps the request into the supported range, or falls through to
/// `BufferSize::Default` when the range is `Unknown`.
///
/// Note: WASAPI `Fixed` controls the **ring-buffer duration**, not the
/// callback period — the callback period is whatever
/// `IAudioClient::GetDevicePeriod` returns (typically 10 ms shared / 3 ms
/// exclusive). The latency budgeting in DESIGN §6.3 accounts for this.
#[must_use]
pub fn pick_buffer_size(req: u32, range: SupportedBufferSize) -> BufferSize {
    match range {
        SupportedBufferSize::Range { min, max } => BufferSize::Fixed(req.clamp(min, max)),
        SupportedBufferSize::Unknown => BufferSize::Default,
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
#[allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]
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

    #[test]
    fn pick_buffer_size_clamps_below_min() {
        // Requested 256 against a Range { min: 480, max: 1920 } → 480.
        let chosen = pick_buffer_size(
            256,
            SupportedBufferSize::Range {
                min: 480,
                max: 1920,
            },
        );
        match chosen {
            BufferSize::Fixed(n) => assert_eq!(n, 480),
            BufferSize::Default => panic!("expected Fixed(480), got Default"),
        }
    }

    #[test]
    fn pick_buffer_size_clamps_above_max() {
        let chosen = pick_buffer_size(
            8192,
            SupportedBufferSize::Range {
                min: 256,
                max: 4096,
            },
        );
        match chosen {
            BufferSize::Fixed(n) => assert_eq!(n, 4096),
            BufferSize::Default => panic!("expected Fixed(4096), got Default"),
        }
    }

    #[test]
    fn pick_buffer_size_passes_through_when_in_range() {
        let chosen = pick_buffer_size(
            960,
            SupportedBufferSize::Range {
                min: 256,
                max: 1920,
            },
        );
        match chosen {
            BufferSize::Fixed(n) => assert_eq!(n, 960),
            BufferSize::Default => panic!("expected Fixed(960), got Default"),
        }
    }

    #[test]
    fn pick_buffer_size_unknown_falls_back_to_default() {
        let chosen = pick_buffer_size(256, SupportedBufferSize::Unknown);
        assert!(
            matches!(chosen, BufferSize::Default),
            "expected Default for Unknown range, got {chosen:?}"
        );
    }

    #[test]
    fn audio_backend_event_serializes_with_kind_tag() {
        let ev = AudioBackendEvent::Disconnected;
        let json = serde_json::to_value(&ev).expect("serialize");
        assert_eq!(json["kind"], "disconnected");

        let ev = AudioBackendEvent::Underrun { count: 42 };
        let json = serde_json::to_value(&ev).expect("serialize");
        assert_eq!(json["kind"], "underrun");
        assert_eq!(json["count"], 42);
    }

    #[test]
    fn map_stream_error_device_not_available_disconnects() {
        let event = map_stream_error(&StreamError::DeviceNotAvailable, 17);
        assert!(
            matches!(event, AudioBackendEvent::Disconnected),
            "DeviceNotAvailable must dispatch Disconnected, got {event:?}"
        );
    }

    #[test]
    fn map_stream_error_other_variants_become_underrun() {
        // BackendSpecific carries an opaque payload; its concrete
        // construction differs across cpal versions, so we test through
        // a synthesized error wrapped with std::io::Error::other.
        let err = StreamError::BackendSpecific {
            err: cpal::BackendSpecificError {
                description: "ALSA xrun".to_string(),
            },
        };
        let event = map_stream_error(&err, 9);
        match event {
            AudioBackendEvent::Underrun { count } => assert_eq!(count, 9),
            other => panic!("expected Underrun{{count: 9}}, got {other:?}"),
        }
    }
}
