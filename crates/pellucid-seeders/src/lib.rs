//! pellucid-seeders — atomic publish + scheduler + per-source
//! seeder modules.
//!
//! The atomic-publish layer is the load-bearing primitive every
//! seeder calls — it owns the lock → validate → stage →
//! promote → release dance from SPEC-001 §7.4. Scheduler +
//! domain seeders land in T3.6 / T3.7 / T3.8 on top of this
//! foundation.

pub mod atomic_publish;
pub mod aviation;
pub mod climate;
pub mod conflict;
pub mod cyber;
pub mod economic;
pub mod energy;
pub mod envelope;
pub mod infra;
pub mod intel;
pub mod locks;
pub mod logistics;
pub mod maritime;
pub mod markets;
pub mod military;
pub mod prediction;
pub mod registry;
pub mod sanctions;
pub mod scheduler;
pub mod technology;
pub mod theater_posture;
pub mod trade;

pub use atomic_publish::{
    atomic_publish, PublishError, PublishOutcome, DEFAULT_LOCK_LEASE, SEED_META_MIN_TTL_MS,
    STAGING_TTL_MS,
};
pub use envelope::{EnvelopeError, SeedEnvelope, SeedMeta, MAX_ENVELOPE_BYTES};
pub use locks::{acquire_seed_lock, current_holder, release_seed_lock, LockError, LockOutcome};

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
        let v = version();
        assert!(!v.is_empty());
        assert!(v.contains('.'));
    }

    #[test]
    fn version_matches_workspace() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }
}
