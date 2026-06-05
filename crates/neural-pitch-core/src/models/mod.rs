//! Phase 2.2 — neural model resolver.
//!
//! Two submodules:
//!
//! * [`manifest`] — strongly-typed view of the workspace `models.toml`
//!   manifest (a `schema_version` guard plus a list of [`ManifestEntry`]).
//! * [`resolver`] — the [`ensure_model`] entry-point that locates the
//!   manifest, verifies any cached ONNX blob's sha256, and (in later
//!   phases) fetches missing models with a lock + atomic-rename dance.
//!
//! Phase 2.2 ships the manifest parser and the on-disk verification path;
//! the actual network fetch is gated behind the `live-fetch` feature and
//! lands in Phase 2.5/3.

pub mod manifest;
pub mod resolver;

pub use manifest::{Manifest, ManifestEntry};
pub use resolver::{PeekResult, ResolverError, ensure_model, manifest_path, peek};
