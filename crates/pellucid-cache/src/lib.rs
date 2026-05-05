//! pellucid-cache — SQLite-backed KV cache with stampede coalescing,
//! negative-result sentinel, and batch reads. SPEC-001 §7.
//!
//! Replaces the original WorldMonitor `server/_shared/redis.ts` Redis
//! cache layer. Public surface:
//!
//! - [`KvCache`] — pool wrapper exposing the cache primitives.
//! - [`cached_fetch_json`] — stampede-protected single-key fetch with
//!   negative sentinel + envelope writeback (SPEC-001 §7.2 + §7.3).
//! - [`get_cached_json_batch`] — single-transaction batch read
//!   replacing `getCachedJsonBatch` from the original codebase.
//! - [`set_cached_json`] / [`set_negative_sentinel`] — direct writers
//!   used by seeders and integration tests.
//!
//! Stampede semantics: concurrent calls to `cached_fetch_json` with the
//! same key are coalesced via an in-flight registry keyed on `cache_key`.
//! Exactly one fetcher invocation runs per key per pending window; every
//! awaiter receives a clone of the same result.
//!
//! Negative semantics: a `null` upstream result is recorded as a row
//! with `is_negative = 1` and a short TTL (default 120s, overridable).
//! Subsequent reads inside that window return [`CacheHit::NegativeSentinel`]
//! without firing the upstream fetcher again.

pub mod batch;
pub mod coalesce;
pub mod kv;
pub mod negative;
pub mod rate_limit;

pub use batch::{BatchHit, get_cached_json_batch};
pub use coalesce::{CoalesceRegistry, cached_fetch_json};
pub use kv::{CacheHit, KvCache, get_cached_json, set_cached_json, set_negative_sentinel};
pub use negative::DEFAULT_NEGATIVE_TTL_MS;
pub use rate_limit::{
    BucketConfig, BucketKind, RateLimitConfig, RateLimitDecision, check_rate_limit,
};

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
