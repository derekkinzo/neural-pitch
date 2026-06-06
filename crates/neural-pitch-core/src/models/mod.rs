//! Neural model resolver.
//!
//! Two submodules:
//!
//! * [`manifest`] — strongly-typed view of the workspace `models.toml`
//!   manifest (a `schema_version` guard plus a list of [`ManifestEntry`]).
//! * [`resolver`] — the [`ensure_model`] entry-point that locates the
//!   manifest and verifies any cached ONNX blob's sha256.
//!
//! Provides the `models.toml` manifest parser and the on-disk sha256
//! verification path. Live network fetch is not currently implemented.

pub mod manifest;
pub mod resolver;

pub use manifest::{Manifest, ManifestEntry};
pub use resolver::{PeekResult, ResolverError, ensure_model, manifest_path, peek};
