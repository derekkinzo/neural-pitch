//! Strongly-typed view of the workspace `models.toml` manifest.
//!
//! The manifest parser sits in front of the resolver algorithm: it
//! round-trips `[[models]]` blocks into [`ManifestEntry`]s and enforces
//! the `schema_version` exact-match guard so a v2 manifest fails fast
//! rather than silently reinterpreting v1 fields.

use std::path::Path;

use serde::Deserialize;

/// A single `[[models]]` entry parsed from `models.toml`.
///
/// Fields mirror the manifest 1:1; placeholder values (`url = ""`,
/// `sha256 = "0000…"`) are tolerated by the parser but cause
/// [`super::resolver::ensure_model`] to surface
/// [`super::resolver::ResolverError::NotConfigured`].
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ManifestEntry {
    /// Stable model identifier, e.g. `"pesto-v1"`.
    pub name: String,
    /// HTTPS URL where the ONNX blob is hosted (empty == not yet configured).
    pub url: String,
    /// Lower-case hex sha256 of the resolved blob.
    pub sha256: String,
    /// Expected file size in bytes (informational; the hash is the source of truth).
    pub size_bytes: u64,
    /// SPDX license identifier, e.g. `"LGPL-3.0-or-later"`.
    pub license: String,
    /// Models that ship in-tree under `crates/neural-pitch-core/assets/`
    /// instead of being downloaded by the resolver. The resolver short-
    /// circuits the URL/SHA pipeline for `bundled = true` entries; the
    /// SHA is still recorded for provenance tracking.
    #[serde(default)]
    pub bundled: bool,
}

/// All-zeros placeholder sha256 used by the workspace `models.toml`
/// before a real blob is published. The resolver treats this hash as
/// "not configured" and surfaces
/// [`super::resolver::ResolverError::NotConfigured`].
pub const PLACEHOLDER_SHA256: &str =
    "0000000000000000000000000000000000000000000000000000000000000000";

impl ManifestEntry {
    /// `true` when the manifest entry is still a placeholder — empty URL
    /// or the all-zeros dummy sha256. The resolver maps this onto
    /// [`super::resolver::ResolverError::NotConfigured`]. Bundled
    /// entries (`bundled = true`) are never placeholders even though
    /// their URL field is informational only.
    pub fn is_placeholder(&self) -> bool {
        if self.bundled {
            return false;
        }
        self.url.is_empty() || self.sha256 == PLACEHOLDER_SHA256
    }
}

/// Top-level `models.toml` document.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Manifest {
    /// Exact-match schema guard. The current manifest is `schema_version = 1`.
    pub schema_version: u32,
    /// All `[[models]]` entries, preserving manifest order.
    #[serde(default)]
    pub models: Vec<ManifestEntry>,
}

/// The single supported `schema_version` value. Bumping this is a
/// breaking change.
pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

impl Manifest {
    /// Parse a TOML manifest from a string.
    ///
    /// Rejects manifests whose `schema_version` is not exactly
    /// [`SUPPORTED_SCHEMA_VERSION`]. The check is intentionally exact-match
    /// (not `>=`) so a v2 manifest cannot be mis-interpreted as v1 — that
    /// would silently feed reordered or renamed fields into the resolver.
    ///
    /// Validates each entry's `url` and `sha256`:
    /// * `url` MUST be empty (placeholder) or start with `https://` — any
    ///   `http://`, `file://`, or other scheme is rejected. This is the
    ///   load-bearing supply-chain guard before live-fetch lands; without
    ///   it, a typo or HTTP mirror redirect could silently downgrade the
    ///   transport.
    /// * `sha256` is normalised to lower-case, then validated to be either
    ///   the all-zeros placeholder or 64 lower-case hex characters. A
    ///   malformed digest is rejected so the on-disk equality compare
    ///   cannot silently never-match.
    ///
    /// # Errors
    ///
    /// Returns a [`super::resolver::ResolverError::ManifestParse`] when the
    /// TOML is malformed, the schema version is unsupported, or any entry
    /// fails the URL / sha256 validation rules.
    pub fn from_toml_str(src: &str) -> Result<Self, super::resolver::ResolverError> {
        let mut parsed: Self = toml::from_str(src)
            .map_err(|e| super::resolver::ResolverError::ManifestParse(e.to_string()))?;
        if parsed.schema_version != SUPPORTED_SCHEMA_VERSION {
            return Err(super::resolver::ResolverError::ManifestParse(format!(
                "manifest schema_version unsupported: expected {SUPPORTED_SCHEMA_VERSION}, got {}",
                parsed.schema_version
            )));
        }
        for entry in &mut parsed.models {
            // URL: must be empty (placeholder) or https://. We reject http
            // (cleartext), file:// (host-local — must be a separate code
            // path), and anything else.
            if !entry.url.is_empty() && !entry.url.starts_with("https://") {
                return Err(super::resolver::ResolverError::ManifestParse(format!(
                    "non-https url for model {name}: {url}",
                    name = entry.name,
                    url = entry.url,
                )));
            }
            // sha256: normalise to lower-case so the on-disk equality
            // compare in `ensure_model` is direction-independent. Also
            // validate the shape so a typo doesn't silently never match.
            entry.sha256 = entry.sha256.to_ascii_lowercase();
            if entry.sha256 != PLACEHOLDER_SHA256
                && !(entry.sha256.len() == 64
                    && entry.sha256.bytes().all(|b| b.is_ascii_hexdigit()))
            {
                return Err(super::resolver::ResolverError::ManifestParse(format!(
                    "invalid sha256 for model {name}: must be 64 hex chars or the placeholder",
                    name = entry.name,
                )));
            }
        }
        Ok(parsed)
    }

    /// Read + parse a manifest from disk.
    ///
    /// # Errors
    ///
    /// Returns a [`super::resolver::ResolverError::ManifestNotFound`] if the
    /// path does not exist, or the propagated parse error.
    pub fn read_from_path(path: &Path) -> Result<Self, super::resolver::ResolverError> {
        if !path.exists() {
            return Err(super::resolver::ResolverError::ManifestNotFound(
                path.to_path_buf(),
            ));
        }
        let src = std::fs::read_to_string(path)?;
        Self::from_toml_str(&src)
    }

    /// Look up a `[[models]]` entry by name.
    pub fn entry(&self, name: &str) -> Option<&ManifestEntry> {
        self.models.iter().find(|m| m.name == name)
    }
}
