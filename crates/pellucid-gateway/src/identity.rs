//! Request-identity types threaded through `axum::http::Extensions`.
//!
//! Stages 5 (Clerk) and 6 (API key) populate one of [`ClientIdentity`]
//! or [`ApiIdentity`]; stage 7 (entitlement) consumes whichever was
//! installed. Stages 8 + 9 (rate limit) consume the IP that
//! [`RequestIdentity`] carries so the buckets key on the actual caller.

use std::net::IpAddr;

use crate::traits::Tier;

/// Caller authenticated via Clerk. Inserted by stage 5.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientIdentity {
    /// Clerk user identifier.
    pub user_id: String,
    /// Clerk session identifier.
    pub session_id: String,
    /// Tier resolved from the entitlement check (stage 7). Optional
    /// because stage 5 runs *before* the entitlement check; the
    /// gateway re-inserts the same struct with `tier = Some(...)`
    /// once stage 7 has decided.
    pub tier: Option<Tier>,
}

/// Caller authenticated via API key. Inserted by stage 6.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApiIdentity {
    /// Stable identifier the API key resolved to.
    pub identity: String,
    /// Tier the key holder is entitled to.
    pub tier: Tier,
}

/// IP + identity envelope inserted at the very top of the pipeline so
/// every later stage can borrow it without re-parsing headers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RequestIdentity {
    /// Caller IP. Read from `x-forwarded-for` (first hop) or the
    /// connection peer addr.
    pub ip: IpAddr,
    /// Clerk identity if present.
    pub clerk: Option<ClientIdentity>,
    /// API key identity if present. At most one of `clerk` / `api`
    /// is populated for a given request.
    pub api: Option<ApiIdentity>,
}

impl RequestIdentity {
    /// Construct an anonymous identity for `ip`.
    #[must_use]
    pub fn anonymous(ip: IpAddr) -> Self {
        Self {
            ip,
            clerk: None,
            api: None,
        }
    }

    /// `true` iff either Clerk or API-key auth populated.
    #[must_use]
    pub fn is_authenticated(&self) -> bool {
        self.clerk.is_some() || self.api.is_some()
    }

    /// Effective tier: highest of Clerk-derived (if entitlement has
    /// run) and API-key tier; falls back to [`Tier::Anonymous`] when
    /// neither is set.
    #[must_use]
    pub fn effective_tier(&self) -> Tier {
        let clerk_tier = self
            .clerk
            .as_ref()
            .and_then(|c| c.tier)
            .unwrap_or(Tier::Anonymous);
        let api_tier = self.api.as_ref().map(|a| a.tier).unwrap_or(Tier::Anonymous);
        if clerk_tier.rank() >= api_tier.rank() {
            clerk_tier
        } else {
            api_tier
        }
    }

    /// User identifier the entitlement stage queries against. Falls
    /// back to a stable string derived from the IP for anonymous
    /// callers so the entitlement service still sees a key.
    #[must_use]
    pub fn user_key(&self) -> String {
        if let Some(c) = self.clerk.as_ref() {
            return format!("clerk:{}", c.user_id);
        }
        if let Some(a) = self.api.as_ref() {
            return format!("apikey:{}", a.identity);
        }
        format!("ip:{}", self.ip)
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn loopback() -> IpAddr {
        IpAddr::from([127, 0, 0, 1])
    }

    #[test]
    fn anonymous_has_no_auth() {
        let r = RequestIdentity::anonymous(loopback());
        assert!(!r.is_authenticated());
        assert_eq!(r.effective_tier(), Tier::Anonymous);
        assert_eq!(r.user_key(), format!("ip:{}", loopback()));
    }

    #[test]
    fn clerk_user_key_uses_clerk_prefix() {
        let r = RequestIdentity {
            ip: loopback(),
            clerk: Some(ClientIdentity {
                user_id: "u1".into(),
                session_id: "s1".into(),
                tier: Some(Tier::Tier1),
            }),
            api: None,
        };
        assert!(r.is_authenticated());
        assert_eq!(r.effective_tier(), Tier::Tier1);
        assert_eq!(r.user_key(), "clerk:u1");
    }

    #[test]
    fn api_key_user_key_uses_apikey_prefix() {
        let r = RequestIdentity {
            ip: loopback(),
            clerk: None,
            api: Some(ApiIdentity {
                identity: "client-42".into(),
                tier: Tier::Tier2,
            }),
        };
        assert_eq!(r.user_key(), "apikey:client-42");
        assert_eq!(r.effective_tier(), Tier::Tier2);
    }

    #[test]
    fn effective_tier_picks_highest_when_both_present() {
        let r = RequestIdentity {
            ip: loopback(),
            clerk: Some(ClientIdentity {
                user_id: "u".into(),
                session_id: "s".into(),
                tier: Some(Tier::Free),
            }),
            api: Some(ApiIdentity {
                identity: "k".into(),
                tier: Tier::Tier2,
            }),
        };
        assert_eq!(r.effective_tier(), Tier::Tier2);
    }
}
