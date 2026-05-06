//! pellucid-db — SQLite handle pool, migrations, and pragma enforcement
//! for every Pellucid binary that needs a canonical store. SPEC-001 §6.
//!
//! Public surface:
//!
//! - [`Pool`] — type alias for `sqlx::SqlitePool` so dependent crates
//!   refer to a single name.
//! - [`open`] / [`open_in_memory`] — async constructors that enforce
//!   the spec-mandated PRAGMAs and run migrations.
//! - [`migrate`] — applies the bundled migrations directory; idempotent.
//! - [`pragmas`] — exposes the canonical PRAGMA list as a slice for
//!   diagnostic / introspection use.

pub mod error;
pub mod migrate;
pub mod pool;

pub use error::DbError;
pub use migrate::migrate;
pub use pool::{open, open_in_memory, pragmas, Pool, SqliteOpenOptions};

/// Returns the crate version string from `CARGO_PKG_VERSION`.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn version_is_set() {
        assert!(!version().is_empty());
        assert!(version().contains('.'));
    }
}
