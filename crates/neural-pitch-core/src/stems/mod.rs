//! HTDemucs ONNX stem-separation surface.
//!
//! This module sits parallel to [`crate::poly`] for the same reason
//! [`crate::poly`] sits parallel to [`crate::pitch`]: a fundamentally
//! different output shape (four named buffers vs. one F0 frame stream)
//! deserves its own surface.
//!
//! All four submodules are gated behind `feature = "neural"` because
//! HTDemucs inference depends on `ort`, the resampler depends on
//! `rubato`, and the download path depends on a TLS HTTP client.
//!
//! # Stems
//!
//! HTDemucs separates a music recording into the standard four stems
//! used across the source-separation literature: `vocals`, `drums`,
//! `bass`, and `other`. The four stems are returned verbatim to the
//! caller, who decides what to do with them (per-stem playback,
//! per-stem transcription, karaoke remix, etc.).

use std::path::{Path, PathBuf};

use thiserror::Error;
use tokio_util::sync::CancellationToken;

pub mod download;
pub mod htdemucs;
pub mod resample;
pub mod segment;

/// HTDemucs ONNX model URL.
///
/// Pinned to a commit SHA on the MIT-licensed StemSplitio
/// redistribution of the original Meta HTDemucs weights so the
/// [`HTDEMUCS_SHA256`] checksum stays valid forever. The fp32 export
/// is parity-verified against the upstream PyTorch model and ships
/// as a single 4-stem `htdemucs.onnx` file.
///
/// Source: <https://huggingface.co/StemSplitio/htdemucs-onnx>
/// License: MIT (matches the original facebookresearch/demucs).
pub const HTDEMUCS_MODEL_URL: &str = "https://huggingface.co/StemSplitio/htdemucs-onnx/resolve/d54ed9eb60e258ea82131c6ee14578628816456a/htdemucs.onnx";

/// SHA-256 of the bytes at [`HTDEMUCS_MODEL_URL`]. Verified after the
/// streamed download completes; mismatch → file is removed and
/// [`StemError::HashMismatch`] is returned.
pub const HTDEMUCS_SHA256: &str =
    "68d0bf16428ef66e692cdff8a9ccf28f1ef3f69440d57e58605a4cc55fcc5e74";

/// Exact byte size of the pinned model blob. The streamed download
/// path reads `Content-Length` from the HTTP response and uses this
/// constant only as a fallback for offline-aware UIs that want to
/// surface "~316 MB" before the network call.
pub const HTDEMUCS_SIZE_BYTES: u64 = 316_446_953;

/// Internal sample rate HTDemucs operates at, in Hertz.
pub const HTDEMUCS_SR_HZ: u32 = 44_100;

/// Output of a single stem-separation pass.
///
/// Stems are returned in 32-bit float PCM at the caller's source
/// sample rate (a re-resample step inside [`StemSeparator::separate`]
/// undoes the internal coercion to [`HTDEMUCS_SR_HZ`]). `channels`
/// mirrors the input: mono input → mono stems; stereo input →
/// stereo stems. The buffer layout is interleaved when
/// `channels == 2`, identical to the existing [`crate::audio`]
/// conventions.
#[derive(Clone, Debug)]
#[must_use]
pub struct StemResult {
    /// Vocals stem.
    pub vocals: Vec<f32>,
    /// Drums stem.
    pub drums: Vec<f32>,
    /// Bass stem.
    pub bass: Vec<f32>,
    /// Other (non-vocal, non-drum, non-bass content).
    pub other: Vec<f32>,
    /// Sample rate of all four stems, in Hertz.
    pub sample_rate_hz: u32,
    /// Channel count of all four stems (1 for mono, 2 for stereo).
    pub channels: u32,
}

/// Errors raised by the stem-separation surface.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum StemError {
    /// The model file does not exist at the expected on-disk path.
    /// Distinct from [`StemError::OfflineFirstUse`] because the model
    /// was previously downloaded but has since been deleted /
    /// renamed / sideloaded into the wrong directory.
    #[error("model not found: {0}")]
    ModelNotFound(PathBuf),

    /// The model has never been downloaded and the first-use download
    /// failed because the network is unavailable. The frontend should
    /// surface both the `url` and the `dest` path verbatim so the user
    /// can manually copy the file over.
    #[error("network unavailable; download {url} manually to {dest}")]
    OfflineFirstUse {
        /// Public URL the model can be downloaded from.
        url: &'static str,
        /// On-disk destination the user should drop the file at.
        dest: PathBuf,
    },

    /// SHA-256 of the downloaded file did not match
    /// [`HTDEMUCS_SHA256`]. The corrupt file is removed before this
    /// error is returned.
    #[error("sha256 mismatch (expected {expected}, got {actual})")]
    HashMismatch {
        /// Expected SHA-256 hex string (compiled into the binary).
        expected: String,
        /// Observed SHA-256 hex string from the downloaded blob.
        actual: String,
    },

    /// Underlying ONNX Runtime error. Stringly-typed to keep `ort`
    /// types out of the public API surface.
    #[error("ort runtime error: {0}")]
    Ort(String),

    /// Filesystem / I/O error during download, hashing, or model
    /// load.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// Caller-supplied configuration was invalid (e.g. unsupported
    /// channel count, zero sample rate, empty input buffer).
    #[error("invalid configuration: {0}")]
    Configuration(String),

    /// The cancellation token fired before separation completed.
    /// Mirrors the [`crate::pipeline::recording`] convention of using
    /// a typed `Cancelled` variant rather than introducing a separate
    /// `CancelledError` type.
    #[error("cancelled")]
    Cancelled,
}

/// HTDemucs ONNX stem separator.
///
/// Constructed via [`StemSeparator::open`] from a model path resolved
/// by [`StemSeparator::ensure_model`]. Holds the underlying
/// `ort::Session` as an opaque box so the public API does not leak
/// `ort` types.
pub struct StemSeparator {
    session: htdemucs::Session,
}

impl StemSeparator {
    /// Resolve the on-disk model path, downloading the HTDemucs ONNX
    /// blob to the per-platform application data directory if it is
    /// not already cached.
    ///
    /// `progress` is invoked with values in `[0.0, 1.0]` as the
    /// streamed download advances. It is not called when the model
    /// is already cached (the function returns immediately).
    pub fn ensure_model<F: FnMut(f32)>(progress: F) -> Result<PathBuf, StemError> {
        let dest = default_models_dir()?;
        download::ensure_at(&dest, progress)
    }

    /// Resolve the on-disk model path under an explicit destination
    /// directory, downloading the blob if it is not already cached.
    ///
    /// This is the variant the Tauri command should call: the Tauri
    /// shell resolves `app.path().app_data_dir()` and passes it in,
    /// so the core crate does not need to depend on `tauri`.
    pub fn ensure_model_at<F: FnMut(f32)>(
        dest_dir: &Path,
        progress: F,
    ) -> Result<PathBuf, StemError> {
        download::ensure_at(dest_dir, progress)
    }

    /// Cancellation-aware variant of [`Self::ensure_model_at`]. The
    /// cancel token is polled inside the streaming-read loop so a
    /// tripped token short-circuits the download mid-blob rather than
    /// waiting for the full ~316 MB to finish.
    pub fn ensure_model_at_with_cancel<F: FnMut(f32)>(
        dest_dir: &Path,
        progress: F,
        cancel: &CancellationToken,
    ) -> Result<PathBuf, StemError> {
        download::ensure_at_with_cancel(dest_dir, progress, cancel)
    }

    /// Open an HTDemucs ONNX session against a previously-cached
    /// model file.
    pub fn open(model_path: &Path) -> Result<Self, StemError> {
        let session = htdemucs::Session::open(model_path)?;
        Ok(Self { session })
    }

    /// Run stem separation on the input audio buffer.
    ///
    /// `input` is interleaved when `channels == 2` and bare-mono when
    /// `channels == 1`. `sample_rate_hz` is the rate of `input`; the
    /// separator internally resamples to [`HTDEMUCS_SR_HZ`] before
    /// inference.
    ///
    /// `progress` is invoked with values in `[0.0, 1.0]` after each
    /// inference segment. `cancel` is checked on entry and once per
    /// segment; firing it returns
    /// [`StemError::Cancelled`] without producing a partial result.
    pub fn separate<F: FnMut(f32)>(
        &mut self,
        input: &[f32],
        sample_rate_hz: u32,
        channels: u32,
        progress: F,
        cancel: &CancellationToken,
    ) -> Result<StemResult, StemError> {
        if cancel.is_cancelled() {
            return Err(StemError::Cancelled);
        }
        if !matches!(channels, 1 | 2) {
            return Err(StemError::Configuration(format!(
                "unsupported channel count: {channels}"
            )));
        }
        if sample_rate_hz == 0 {
            return Err(StemError::Configuration(
                "sample_rate_hz must be greater than zero".to_string(),
            ));
        }
        if input.is_empty() {
            return Ok(StemResult {
                vocals: Vec::new(),
                drums: Vec::new(),
                bass: Vec::new(),
                other: Vec::new(),
                sample_rate_hz,
                channels,
            });
        }

        // 1. Resample the caller buffer to 44.1 kHz interleaved stereo.
        let stereo_44k1 = resample::to_htdemucs_input(input, sample_rate_hz, channels)?;

        // 2. Run the segment loop with overlap-add reconstruction.
        let stems =
            segment::separate_overlap_add(&mut self.session, &stereo_44k1, progress, cancel)?;

        // 3. Re-resample each stem back to the caller's contract.
        let vocals = resample::from_htdemucs_output(&stems.vocals, sample_rate_hz, channels)?;
        let drums = resample::from_htdemucs_output(&stems.drums, sample_rate_hz, channels)?;
        let bass = resample::from_htdemucs_output(&stems.bass, sample_rate_hz, channels)?;
        let other = resample::from_htdemucs_output(&stems.other, sample_rate_hz, channels)?;

        // 4. Pin the per-stem length to the input length to absorb the
        //    one-sample drift the asymmetric resamplers can introduce.
        let target_len = input.len();
        let pin = |mut buf: Vec<f32>| -> Vec<f32> {
            if buf.len() > target_len {
                buf.truncate(target_len);
            } else if buf.len() + 1 == target_len {
                buf.push(0.0);
            }
            buf
        };

        Ok(StemResult {
            vocals: pin(vocals),
            drums: pin(drums),
            bass: pin(bass),
            other: pin(other),
            sample_rate_hz,
            channels,
        })
    }
}

/// Default models cache directory, resolved via `directories::ProjectDirs`.
///
/// Mirrors the existing per-platform layout used elsewhere in the
/// workspace (the Tauri shell resolves the same dir via
/// `app.path().app_data_dir()`).
fn default_models_dir() -> Result<PathBuf, StemError> {
    use directories::ProjectDirs;
    let dirs = ProjectDirs::from("dev", "neural-pitch", "neural-pitch").ok_or_else(|| {
        StemError::Configuration("could not resolve per-platform data dir".to_string())
    })?;
    let mut p = dirs.data_dir().to_path_buf();
    p.push("models");
    Ok(p)
}
