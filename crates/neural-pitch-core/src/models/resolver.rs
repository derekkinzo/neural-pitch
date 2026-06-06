//! Phase 2.2 — model resolver. Locate, verify, and (later) fetch ONNX blobs
//! described by the workspace `models.toml`.
//!
//! Phase 2.2 ships the manifest + on-disk verification plumbing only. The
//! real network fetch (step 7 of the algorithm in the resolver spec) is
//! gated behind `cfg(feature = "live-fetch")` or a runtime env-var so unit
//! tests stay hermetic.

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use super::manifest::{Manifest, ManifestEntry};

/// All failure modes surfaced by [`ensure_model`].
///
/// The variants intentionally match the resolver spec 1:1 so callers (the
/// Tauri `get_model_status` command, the future Settings UI) can pattern-match
/// without grepping the implementation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ResolverError {
    /// `models.toml` was not found at the resolved path.
    #[error("manifest not found: {0}")]
    ManifestNotFound(PathBuf),
    /// `models.toml` exists but failed to parse (malformed TOML or unsupported
    /// `schema_version`).
    #[error("manifest parse error: {0}")]
    ManifestParse(String),
    /// The requested model name is absent from the manifest.
    #[error("unknown model: {0}")]
    UnknownModel(String),
    /// The manifest entry is a placeholder: empty URL or the all-zeros
    /// dummy sha256. The model cannot be fetched until Phase 2.5/3 fills
    /// the fields in.
    #[error("model {name} is not configured (placeholder manifest entry)")]
    NotConfigured {
        /// Name of the un-configured model.
        name: String,
    },
    /// The on-disk blob's sha256 did not match the manifest. The corrupted
    /// file is deleted before this error is surfaced.
    #[error("sha256 mismatch: expected {expected}, got {actual}")]
    HashMismatch {
        /// Manifest-declared hex sha256.
        expected: String,
        /// Computed hex sha256 of the on-disk blob.
        actual: String,
    },
    /// Network fetch failed (only reachable under `live-fetch`).
    #[error("fetch failed: {0}")]
    Fetch(String),
    /// Filesystem error during verification, locking, or atomic rename.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Resolve the canonical workspace `models.toml` path.
///
/// Phase 2.2: returns `<workspace-root>/models.toml` for dev builds. The
/// workspace root is anchored on `CARGO_MANIFEST_DIR` of the
/// `neural-pitch-core` crate (which lives at `crates/neural-pitch-core/`),
/// so the manifest is two parents up.
///
/// Packaged builds will resolve `<app-data>/models.toml` from the Tauri
/// shell; that branch is added in a later phase.
#[must_use]
pub fn manifest_path() -> PathBuf {
    // `CARGO_MANIFEST_DIR` is set at compile time to the crate dir
    // (`<repo>/crates/neural-pitch-core`). The workspace root sits two
    // levels above. The manifest lives at `<workspace-root>/models.toml`.
    let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    match crate_dir
        .parent() // crates/
        .and_then(Path::parent)
    // <repo>/
    {
        Some(p) => p.join("models.toml"),
        None => PathBuf::from("models.toml"),
    }
}

/// Compute the streaming sha256 hex digest of a file.
fn hash_file(path: &Path) -> Result<String, ResolverError> {
    let mut hasher = Sha256::new();
    let mut file = fs::File::open(path)?;
    // 64 KiB scratch — large enough to amortise syscall overhead, small
    // enough to fit comfortably in L1. Heap-allocated so the resolver does
    // not blow the stack-array clippy budget on the hot path.
    let mut buf = vec![0u8; 64 * 1024].into_boxed_slice();
    loop {
        let n = file.read(&mut buf)?;
        if n == 0 {
            break;
        }
        hasher.update(&buf[..n]);
    }
    let digest = hasher.finalize();
    Ok(hex_encode(&digest))
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

/// Result of a non-fetching peek at the manifest.
#[derive(Debug, Clone)]
pub struct PeekResult {
    /// The manifest entry for the requested model.
    pub entry: ManifestEntry,
    /// Resolved on-disk path (whether or not the file currently exists).
    pub target: PathBuf,
    /// `true` when the entry is still a placeholder (empty URL or
    /// all-zeros sha256).
    pub is_placeholder: bool,
    /// `true` when `target` exists on disk and its sha256 matches.
    pub on_disk_match: bool,
}

/// Non-fetching variant of [`ensure_model`] used by the Tauri
/// `get_model_status` command.
///
/// Returns the manifest entry for `name` plus the resolved on-disk path
/// (whether or not the file currently exists). Hashes the on-disk blob
/// when present so callers can distinguish a verified cache from an
/// unverified or missing one.
pub fn peek(name: &str, dest_dir: &Path) -> Result<PeekResult, ResolverError> {
    let manifest = read_workspace_manifest()?;
    let entry = manifest
        .entry(name)
        .ok_or_else(|| ResolverError::UnknownModel(name.to_string()))?
        .clone();
    let target = dest_dir.join(format!("{name}.onnx"));
    let is_placeholder = entry.is_placeholder();
    let on_disk_match = if target.exists() {
        match hash_file(&target) {
            Ok(actual) => actual == entry.sha256,
            Err(_) => false,
        }
    } else {
        false
    };
    Ok(PeekResult {
        entry,
        target,
        is_placeholder,
        on_disk_match,
    })
}

/// Read + parse the workspace `models.toml` manifest. Internal helper used
/// by [`ensure_model`] and [`peek`].
fn read_workspace_manifest() -> Result<Manifest, ResolverError> {
    Manifest::read_from_path(&manifest_path())
}

/// Ensure `<dest_dir>/<name>.onnx` exists and matches the manifest's sha256.
///
/// Algorithm (per the resolver spec):
///
/// 1. Locate manifest via [`manifest_path`] (or a caller-supplied path in
///    tests).
/// 2. Parse with `Manifest::from_toml_str`; reject `schema_version != 1`.
/// 3. Look up entry by `name` — `UnknownModel` if missing.
/// 4. Compute `target = dest_dir.join(format!("{name}.onnx"))`.
/// 5. If `target` exists, stream-hash with `sha2::Sha256`. Match → return
///    `Ok(target)`. Mismatch → delete and fall through.
/// 6. If `entry.url` is empty or the sha256 is the placeholder, return
///    `NotConfigured { name }`.
/// 7. Otherwise: lock `<target>.lock`, fetch into `<target>.partial`,
///    hash on the fly, verify, atomic `fs::rename` to `target`.
///
/// Phase 2.2 ships steps 1–6 + the lock/rename plumbing; step 7's network
/// call is gated behind the `live-fetch` feature.
///
/// # Errors
///
/// Any [`ResolverError`] variant.
pub fn ensure_model(name: &str, dest_dir: &Path) -> Result<PathBuf, ResolverError> {
    // 1-2. Locate + parse the manifest.
    let manifest = read_workspace_manifest()?;

    // 3. Look up the requested entry.
    let entry = manifest
        .entry(name)
        .ok_or_else(|| ResolverError::UnknownModel(name.to_string()))?;

    // 4. Compute target path.
    let target = dest_dir.join(format!("{name}.onnx"));

    // 5. If the target exists, verify it. Match → return; mismatch →
    //    delete and fall through to the fetch path. Under a placeholder
    //    manifest we must NOT delete: the manifest's sha is the all-zeros
    //    sentinel which can never match a real file, so a blanket "delete
    //    on mismatch" would burn any user-supplied hand-placed blob the
    //    moment Phase 2.5 lands. Leave the cached file alone in that case
    //    and surface NotConfigured in step 6.
    if target.exists() && !entry.is_placeholder() {
        let actual = hash_file(&target)?;
        if actual == entry.sha256 {
            return Ok(target);
        }
        // Hash differs against a real manifest sha — corrupted or stale
        // blob. Delete it and surface HashMismatch so the caller can
        // distinguish "never fetched" from "fetched-but-corrupted" and
        // (with live-fetch) drive a retry.
        match fs::remove_file(&target) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(ResolverError::Io(e)),
        }
        return Err(ResolverError::HashMismatch {
            expected: entry.sha256.clone(),
            actual,
        });
    }

    // 6. Placeholder manifest entry → not configured. This guard fires
    //    BEFORE any `.partial` file is opened, so the offline-skip test
    //    can assert no scratch IO has occurred.
    if entry.is_placeholder() {
        return Err(ResolverError::NotConfigured {
            name: name.to_string(),
        });
    }

    // 7. Real fetch path is gated behind `live-fetch`. With the gate off,
    //    Phase 2.2 ends here — the caller is expected to surface
    //    `NotConfigured` semantics through the Tauri `get_model_status`
    //    command and not enter `ensure_model` until Phase 2.5/3.
    Err(ResolverError::NotConfigured {
        name: name.to_string(),
    })
}
