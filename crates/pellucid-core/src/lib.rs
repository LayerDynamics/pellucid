//! pellucid-core
//!
//! Shared types, errors, FNV hashing, envelope schema, time helpers, and
//! identifier types used across the entire Pellucid Cargo workspace.
//!
//! See `docs/specs/SPEC-001-pellucid-stack-rebuild.md` §11 for the role
//! this crate plays in the workspace topology.
//!
//! Pure types only — no I/O, no async runtime. Every dependent crate
//! re-exports these as the canonical building blocks for envelopes,
//! cache-tier headers, ETag computation (FNV-1a), and shared error
//! mapping.

pub mod cache_tier;
pub mod envelope;
pub mod error;
pub mod fnv;
pub mod id;
pub mod seed_meta;
pub mod time;

pub use cache_tier::CacheTier;
pub use envelope::{
    validate_envelope_size, Envelope, EnvelopeMeta, SeedEnvelope, MAX_ENVELOPE_BYTES,
};
pub use error::{Error, Result};
pub use fnv::FnvHasher;
pub use id::RunId;
pub use seed_meta::{SeedMeta, SeedState};
pub use time::now_ms;

/// Returns the crate version string from `CARGO_PKG_VERSION`.
///
/// Used by the universal smoke test (per implementation plan §1.1) so every
/// crate has at least one passing unit test from the moment it is created.
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
        let v = version();
        assert!(!v.is_empty(), "version must not be empty");
        assert!(v.contains('.'), "expected semver with dot, got {v}");
    }

    #[test]
    fn version_matches_workspace() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn re_exports_resolve() {
        // Compile-time proof that the public surface re-exports are
        // reachable through the crate root.
        let _: CacheTier = CacheTier::Fast;
        let _: SeedState = SeedState::Live;
        let _: RunId = RunId::new();
        let _: usize = MAX_ENVELOPE_BYTES;
        let _: Result<()> = Ok(());
    }
}
