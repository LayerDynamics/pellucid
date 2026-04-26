//! pellucid-auth — Clerk JWT verify, entitlement check (T2.3), HMAC
//! sign/verify (T2.4).
//!
//! T2.2 ships the Clerk verifier + JWKS cache. T2.3 (entitlement) and
//! T2.4 (HMAC identity) plug into the same crate as separate modules.

pub mod clerk;
pub mod jwks;

#[cfg(any(test, feature = "test-keys"))]
pub mod test_keys;

pub use clerk::{ClerkJwtVerifier, DEFAULT_LEEWAY_SECS};
pub use jwks::{CachedJwks, JwksCache, JwksError, JwksFetcher, DEFAULT_TTL};

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
        assert!(!v.is_empty(), "version must not be empty");
        assert!(v.contains('.'), "expected semver with dot, got {v}");
    }

    #[test]
    fn version_matches_workspace() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }
}
