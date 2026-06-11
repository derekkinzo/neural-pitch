//! Streamed model download + SHA-256 verification + atomic rename.
//!
//! Concurrency: a static per-destination-path mutex serialises
//! multi-thread callers so two threads do not both stream the same
//! 316 MB blob. The second thread to acquire the lock re-checks the
//! on-disk SHA before redoing the download — the common case after
//! a contended first-use is "already resolved, return immediately".

#![cfg(feature = "neural")]

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use tokio_util::sync::CancellationToken;

use crate::stems::{HTDEMUCS_MODEL_URL, HTDEMUCS_SHA256, StemError};

/// Filename the model is cached under inside the destination
/// directory.
pub const MODEL_FILENAME: &str = "htdemucs.onnx";

/// Filename the partially-downloaded blob is staged under before
/// the atomic rename to [`MODEL_FILENAME`].
pub const PARTIAL_FILENAME: &str = "htdemucs.onnx.partial";

/// Buffer size for the streamed read loop, in bytes. 256 KiB chunks
/// amortise syscall overhead while keeping the progress callback's
/// reporting cadence fine-grained enough for a smooth UI.
const STREAM_CHUNK_BYTES: usize = 256 * 1024;

/// Per-destination-path lock table: serialises concurrent callers
/// asking for the same on-disk file.
fn download_lock_for(dest: &Path) -> Arc<Mutex<()>> {
    static TABLE: parking_lot::Mutex<Option<HashMap<PathBuf, Arc<Mutex<()>>>>> =
        parking_lot::Mutex::new(None);
    let mut guard = TABLE.lock();
    let map = guard.get_or_insert_with(HashMap::new);
    map.entry(dest.to_path_buf())
        .or_insert_with(|| Arc::new(Mutex::new(())))
        .clone()
}

/// Stream-hash an on-disk file with SHA-256, returning the lower-case
/// hex digest. Pulls 64 KiB at a time so the hot path stays L1-friendly.
fn sha256_file(path: &Path) -> std::io::Result<String> {
    let mut hasher = Sha256::new();
    let mut file = fs::File::open(path)?;
    let mut buf = vec![0u8; 64 * 1024].into_boxed_slice();
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    Ok(hex_encode(hasher.finalize().as_slice()))
}

/// Lower-case hex encoding without pulling in an extra crate.
fn hex_encode(bytes: &[u8]) -> String {
    const LUT: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(LUT[(b >> 4) as usize] as char);
        out.push(LUT[(b & 0x0f) as usize] as char);
    }
    out
}

/// Resolve the on-disk model path under `dest_dir`, downloading
/// the HTDemucs ONNX blob if it is not already cached.
///
/// On a cache hit, the existing file's SHA-256 is verified against
/// [`crate::stems::HTDEMUCS_SHA256`]; mismatch → the file is
/// removed and a fresh download is attempted.
///
/// On first use offline, returns
/// [`StemError::OfflineFirstUse`] with the public URL the user
/// should hand-download from and the destination they should drop
/// the file at.
///
/// Convenience wrapper that forwards to [`ensure_at_with_cancel`]
/// with an unset cancellation token.
pub fn ensure_at<F: FnMut(f32)>(dest_dir: &Path, progress: F) -> Result<PathBuf, StemError> {
    ensure_at_with_cancel(dest_dir, progress, &CancellationToken::new())
}

/// Cancellation-aware variant of [`ensure_at`]. Polls `cancel`
/// between every chunk read so a tripped token short-circuits the
/// streamed download instead of waiting for the full ~316 MB blob to
/// finish.
pub fn ensure_at_with_cancel<F: FnMut(f32)>(
    dest_dir: &Path,
    mut progress: F,
    cancel: &CancellationToken,
) -> Result<PathBuf, StemError> {
    if cancel.is_cancelled() {
        return Err(StemError::Cancelled);
    }
    fs::create_dir_all(dest_dir)?;

    let target = dest_dir.join(MODEL_FILENAME);
    let lock = download_lock_for(&target);
    let _guard = lock.lock();

    if cancel.is_cancelled() {
        return Err(StemError::Cancelled);
    }

    // Re-check on disk under the lock — first thread to win the race
    // may have already populated the file.
    if target.exists() {
        let actual = sha256_file(&target)?;
        if actual.eq_ignore_ascii_case(HTDEMUCS_SHA256) {
            return Ok(target);
        }
        // Hash differs — corrupt or stale blob. Remove and re-fetch.
        fs::remove_file(&target).ok();
    }

    download_and_verify(&target, &mut progress, cancel)?;
    Ok(target)
}

/// Run the streamed download → atomic rename → sha verify pipeline
/// against `target`. Splits out so [`ensure_at`] stays linear.
fn download_and_verify<F: FnMut(f32)>(
    target: &Path,
    progress: &mut F,
    cancel: &CancellationToken,
) -> Result<(), StemError> {
    let partial = target.with_file_name(PARTIAL_FILENAME);
    // Best-effort cleanup of any prior partial.
    fs::remove_file(&partial).ok();

    let response = reqwest::blocking::Client::builder()
        .build()
        .map_err(|e| StemError::Configuration(format!("reqwest client: {e}")))?
        .get(HTDEMUCS_MODEL_URL)
        .send();
    let response = match response {
        Ok(r) => r,
        Err(e) if e.is_connect() || e.is_timeout() || e.is_request() => {
            return Err(StemError::OfflineFirstUse {
                url: HTDEMUCS_MODEL_URL,
                dest: target.to_path_buf(),
            });
        }
        Err(e) => return Err(StemError::Configuration(format!("reqwest send: {e}"))),
    };
    if !response.status().is_success() {
        return Err(StemError::Configuration(format!(
            "model download HTTP {}",
            response.status()
        )));
    }

    let total_bytes = response.content_length();
    let mut bytes_so_far: u64 = 0;
    let mut hasher = Sha256::new();

    {
        let mut writer = std::io::BufWriter::new(fs::File::create(&partial)?);
        let mut reader = response;
        let mut buf = vec![0u8; STREAM_CHUNK_BYTES].into_boxed_slice();
        loop {
            // Poll the cancel token at every chunk boundary so a tripped
            // token short-circuits within ~one STREAM_CHUNK_BYTES read of
            // wall-clock latency. The partial file is removed before the
            // typed Cancelled error is surfaced.
            if cancel.is_cancelled() {
                fs::remove_file(&partial).ok();
                return Err(StemError::Cancelled);
            }
            let n = reader.read(&mut buf)?;
            if n == 0 {
                break;
            }
            hasher.update(&buf[..n]);
            writer.write_all(&buf[..n])?;
            bytes_so_far += n as u64;
            if let Some(total) = total_bytes
                && total > 0
            {
                let frac = (bytes_so_far as f64 / total as f64) as f32;
                progress(frac.clamp(0.0, 1.0));
            }
        }
        writer.flush()?;
        writer
            .into_inner()
            .map_err(std::io::IntoInnerError::into_error)?
            .sync_all()?;
    }

    let actual = hex_encode(hasher.finalize().as_slice());
    if !actual.eq_ignore_ascii_case(HTDEMUCS_SHA256) {
        fs::remove_file(&partial).ok();
        return Err(StemError::HashMismatch {
            expected: HTDEMUCS_SHA256.to_string(),
            actual,
        });
    }

    fs::rename(&partial, target)?;

    // POSIX: enforce 0644 explicitly so the cache file is world-readable
    // regardless of the worker's umask (containers with umask 0077
    // would otherwise produce 0600; CI runners with umask 0002 would
    // produce 0664). Windows has no equivalent and skips the call.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(target, std::fs::Permissions::from_mode(0o644));
    }

    // POSIX: durably commit the directory entry so a power-loss between
    // rename-return and the kernel's metadata commit cannot lose the
    // freshly-renamed file. Windows directory handles do not fsync the
    // same way, so the call is skipped there.
    #[cfg(unix)]
    {
        if let Some(parent) = target.parent()
            && let Ok(dir) = fs::File::open(parent)
        {
            let _ = dir.sync_all();
        }
    }

    progress(1.0);
    Ok(())
}
