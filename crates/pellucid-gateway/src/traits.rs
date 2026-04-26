//! Pluggable trait surfaces the gateway's stages depend on.
//!
//! Each trait has a real, fully-functional default implementation
//! (used by tests + the M0 web shard) plus a named slot the M1 crates
//! plug into:
//!
//! - [`ClerkVerifier`] — T2.2 supplies `pellucid_auth::ClerkJwtVerifier`.
//! - [`ApiKeyStore`] — T5 supplies a SQLite-backed lookup.
//! - [`EntitlementChecker`] — T2.3 supplies the cache → Convex
//!   fallback (with the H2 three-arm decision).
//!
//! Stages call these traits via `Arc<dyn ...>`, so swapping in the
//! real implementation is a one-line change in `GatewayConfig`.

use async_trait::async_trait;
use thiserror::Error;

/// Tier classification for a route or a caller. Matches the four-tier
/// scheme from SPEC-001 §13.4 / OP-10.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub enum Tier {
    /// Anonymous (no auth required).
    Anonymous,
    /// Free signed-in user.
    Free,
    /// Paid tier 1.
    Tier1,
    /// Paid tier 2 (premium endpoints).
    Tier2,
}

impl Tier {
    /// Numeric rank (0..=3). Higher tiers strictly satisfy lower
    /// requirements: caller `Tier::Tier2` satisfies `required ==
    /// Tier::Free` etc.
    #[must_use]
    pub const fn rank(self) -> u8 {
        match self {
            Self::Anonymous => 0,
            Self::Free => 1,
            Self::Tier1 => 2,
            Self::Tier2 => 3,
        }
    }

    /// `true` iff `self.rank() >= required.rank()`.
    #[must_use]
    pub const fn satisfies(self, required: Tier) -> bool {
        self.rank() >= required.rank()
    }
}

/// Claims returned by [`ClerkVerifier::verify`]. Real Clerk tokens
/// carry many more claims; the gateway only consumes these four.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClerkClaims {
    /// Clerk user identifier (`sub`).
    pub user_id: String,
    /// Session identifier (`sid`).
    pub session_id: String,
    /// `exp` epoch seconds.
    pub expires_at: i64,
    /// `iss` claim.
    pub issuer: String,
}

/// Errors a Clerk verifier may raise. Mapped to 401 by the gateway
/// stage.
#[derive(Debug, Error)]
pub enum ClerkVerifyError {
    /// Token is structurally invalid.
    #[error("malformed Clerk token")]
    Malformed,
    /// Token expired.
    #[error("Clerk token expired")]
    Expired,
    /// Signature did not validate against the JWKS.
    #[error("Clerk token signature invalid")]
    BadSignature,
    /// Underlying network/JWKS lookup failed.
    #[error("Clerk JWKS fetch failed: {0}")]
    Jwks(String),
}

/// Pluggable Clerk JWT verifier.
#[async_trait]
pub trait ClerkVerifier: Send + Sync + std::fmt::Debug {
    /// Validate `token` and return the claims on success.
    async fn verify(&self, token: &str) -> Result<ClerkClaims, ClerkVerifyError>;
}

/// Default verifier used by tests + crates that have not yet wired in
/// the real T2.2 implementation. Rejects every token with
/// `Malformed` so any tier-gated route is closed-by-default.
#[derive(Debug, Default)]
pub struct NoopClerkVerifier;

#[async_trait]
impl ClerkVerifier for NoopClerkVerifier {
    async fn verify(&self, _token: &str) -> Result<ClerkClaims, ClerkVerifyError> {
        Err(ClerkVerifyError::Malformed)
    }
}

/// Decision returned by [`ApiKeyStore::lookup`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiKeyDecision {
    /// Key is valid for the given identity + tier.
    Allow {
        /// Stable identity associated with the key.
        identity: String,
        /// Tier the key holder is entitled to.
        tier: Tier,
    },
    /// Key was found but has been revoked.
    Revoked,
    /// Key was not present in the store.
    Unknown,
}

/// Pluggable API-key lookup.
#[async_trait]
pub trait ApiKeyStore: Send + Sync + std::fmt::Debug {
    /// Lookup `key` and return the decision.
    async fn lookup(&self, key: &str) -> ApiKeyDecision;
}

/// Default store used when the deployment does not expose API-key
/// auth. Returns `Unknown` for every key so any API-key-gated route
/// closes by default.
#[derive(Debug, Default)]
pub struct NoopApiKeyStore;

#[async_trait]
impl ApiKeyStore for NoopApiKeyStore {
    async fn lookup(&self, _key: &str) -> ApiKeyDecision {
        ApiKeyDecision::Unknown
    }
}

/// Three-arm entitlement decision from SPEC-001 §11.2 + §24 (H2 fix).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntitlementDecision {
    /// Caller is entitled to the required tier.
    Allow {
        /// Caller's effective tier.
        effective_tier: Tier,
    },
    /// Caller is genuinely under-tier — produce a 403.
    Deny {
        /// Caller's effective tier (always strictly less than required).
        effective_tier: Tier,
    },
    /// Both the cache and the upstream entitlement service are
    /// unreachable. The H2 fix routes this to 503 + Retry-After
    /// instead of 403, so the webview can show an outage banner
    /// rather than a misleading upgrade prompt.
    UpstreamDown {
        /// Suggested retry delay in seconds.
        retry_after_secs: u32,
    },
}

/// Pluggable entitlement checker.
#[async_trait]
pub trait EntitlementChecker: Send + Sync + std::fmt::Debug {
    /// Return the entitlement decision for `(user_id, required)`.
    async fn check(&self, user_id: &str, required: Tier) -> EntitlementDecision;
}

/// Allow-everything entitlement checker. Used by integration tests and
/// (intentionally) the M0 web shard which has no entitlement service.
#[derive(Debug, Default)]
pub struct AlwaysAllowEntitlement;

#[async_trait]
impl EntitlementChecker for AlwaysAllowEntitlement {
    async fn check(&self, _user_id: &str, _required: Tier) -> EntitlementDecision {
        EntitlementDecision::Allow {
            effective_tier: Tier::Tier2,
        }
    }
}

/// Deny-everything entitlement checker. Used to verify failure paths.
#[derive(Debug, Default)]
pub struct AlwaysDenyEntitlement;

#[async_trait]
impl EntitlementChecker for AlwaysDenyEntitlement {
    async fn check(&self, _user_id: &str, _required: Tier) -> EntitlementDecision {
        EntitlementDecision::Deny {
            effective_tier: Tier::Free,
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn tier_rank_is_monotonic() {
        assert!(Tier::Anonymous.rank() < Tier::Free.rank());
        assert!(Tier::Free.rank() < Tier::Tier1.rank());
        assert!(Tier::Tier1.rank() < Tier::Tier2.rank());
    }

    #[test]
    fn tier_satisfies_is_inclusive() {
        for t in [Tier::Anonymous, Tier::Free, Tier::Tier1, Tier::Tier2] {
            assert!(t.satisfies(t), "{t:?} must satisfy itself");
        }
        assert!(Tier::Tier2.satisfies(Tier::Free));
        assert!(!Tier::Free.satisfies(Tier::Tier2));
    }

    #[tokio::test]
    async fn noop_clerk_verifier_rejects_every_token() {
        let v = NoopClerkVerifier;
        let err = v.verify("any").await.unwrap_err();
        assert!(matches!(err, ClerkVerifyError::Malformed));
    }

    #[tokio::test]
    async fn noop_api_key_store_returns_unknown() {
        let s = NoopApiKeyStore;
        assert_eq!(s.lookup("anything").await, ApiKeyDecision::Unknown);
    }

    #[tokio::test]
    async fn always_allow_entitlement_returns_tier2() {
        let c = AlwaysAllowEntitlement;
        let d = c.check("user", Tier::Tier1).await;
        assert!(matches!(
            d,
            EntitlementDecision::Allow {
                effective_tier: Tier::Tier2
            }
        ));
    }

    #[tokio::test]
    async fn always_deny_entitlement_returns_free() {
        let c = AlwaysDenyEntitlement;
        let d = c.check("user", Tier::Tier1).await;
        assert!(matches!(
            d,
            EntitlementDecision::Deny {
                effective_tier: Tier::Free
            }
        ));
    }
}
