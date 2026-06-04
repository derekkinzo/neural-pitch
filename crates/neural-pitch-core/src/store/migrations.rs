//! Refinery-driven migration runner for the `store` module.
//!
//! `embed_migrations!` walks the `migrations/` subdirectory at compile time
//! and statically embeds every `V000N__*.sql` file into the binary. Refinery
//! refuses to apply out-of-order or modified migrations, so accidental edits
//! to V0001 are CI-blocked rather than silently corrupting databases in the
//! wild.

use rusqlite::Connection;

use super::error::StoreError;

// `embed_migrations!` resolves its path argument relative to `CARGO_MANIFEST_DIR`,
// not the source file, so we point at `src/store/migrations/`.
mod embedded {
    refinery::embed_migrations!("src/store/migrations");
}

/// Run every embedded migration that has not yet been applied to `conn`.
pub(super) fn run(conn: &mut Connection) -> Result<(), StoreError> {
    // `StoreError::Migration` carries `#[from]` for `refinery::Error`, so
    // the `?` lifts the conversion automatically.
    embedded::migrations::runner().run(conn).map(|_| ())?;
    Ok(())
}
