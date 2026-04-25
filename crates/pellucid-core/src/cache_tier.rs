//! Cache tier definitions — the six-tier hierarchy from SPEC-001 §7.1
//! plus a `NoStore` opt-out for live tracking endpoints (vessels, etc.).
//! Every tier maps to a `Cache-Control` header value and a `CDN-Cache-
//! Control` value used by the gateway's stage 14.

use core::fmt;
use core::str::FromStr;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};

/// A cache tier categorizes how aggressively a response should be cached
/// at the CDN, browser, and edge. The `s_maxage`, `stale_while_revalidate`
/// and `stale_if_error` durations are spec-mandated and **must not drift**
/// without an explicit SPEC-001 update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CacheTier {
    /// 300s s-maxage / 60s SWR / 1200s SIE — aviation status, OREF, air quality.
    Fast,
    /// 600s / 120s / 1800s — market quotes, crypto, hyperliquid.
    Medium,
    /// 1800s / 300s / 7200s — ACLED, cyber threats, climate news.
    Slow,
    /// 900s / 60s / 1800s — premium supply-chain.
    SlowBrowser,
    /// 3600s / 600s / 28800s — ETF flows, airport delays.
    Static,
    /// 86400s / 3600s / 172800s — critical minerals, tariffs.
    Daily,
    /// 0/0/0 — vessels, aircraft tracking. Emits `Cache-Control: no-store`.
    NoStore,
}

/// Header values for one cache tier. Surfaced as a struct so the gateway's
/// header-merge stage can compose them with the per-RPC override map.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TierHeaders {
    /// `s-maxage` directive in seconds.
    pub s_maxage: u32,
    /// `stale-while-revalidate` directive in seconds.
    pub stale_while_revalidate: u32,
    /// `stale-if-error` directive in seconds.
    pub stale_if_error: u32,
}

impl CacheTier {
    /// Returns the spec-mandated header values for this tier.
    #[must_use]
    pub const fn headers(self) -> TierHeaders {
        match self {
            Self::Fast => TierHeaders {
                s_maxage: 300,
                stale_while_revalidate: 60,
                stale_if_error: 1200,
            },
            Self::Medium => TierHeaders {
                s_maxage: 600,
                stale_while_revalidate: 120,
                stale_if_error: 1800,
            },
            Self::Slow => TierHeaders {
                s_maxage: 1800,
                stale_while_revalidate: 300,
                stale_if_error: 7200,
            },
            Self::SlowBrowser => TierHeaders {
                s_maxage: 900,
                stale_while_revalidate: 60,
                stale_if_error: 1800,
            },
            Self::Static => TierHeaders {
                s_maxage: 3600,
                stale_while_revalidate: 600,
                stale_if_error: 28800,
            },
            Self::Daily => TierHeaders {
                s_maxage: 86400,
                stale_while_revalidate: 3600,
                stale_if_error: 172_800,
            },
            Self::NoStore => TierHeaders {
                s_maxage: 0,
                stale_while_revalidate: 0,
                stale_if_error: 0,
            },
        }
    }

    /// Returns the canonical `Cache-Control` header value for this tier.
    #[must_use]
    pub fn cache_control(self) -> String {
        if matches!(self, Self::NoStore) {
            return "no-store".to_string();
        }
        let h = self.headers();
        format!(
            "public, s-maxage={}, stale-while-revalidate={}, stale-if-error={}",
            h.s_maxage, h.stale_while_revalidate, h.stale_if_error
        )
    }

    /// Returns the kebab-case string form used in serialized configuration
    /// (env vars `CACHE_TIER_OVERRIDE_<RPC>`, JSON config, etc.).
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Fast => "fast",
            Self::Medium => "medium",
            Self::Slow => "slow",
            Self::SlowBrowser => "slow-browser",
            Self::Static => "static",
            Self::Daily => "daily",
            Self::NoStore => "no-store",
        }
    }
}

impl fmt::Display for CacheTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for CacheTier {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self> {
        match s {
            "fast" => Ok(Self::Fast),
            "medium" => Ok(Self::Medium),
            "slow" => Ok(Self::Slow),
            "slow-browser" => Ok(Self::SlowBrowser),
            "static" => Ok(Self::Static),
            "daily" => Ok(Self::Daily),
            "no-store" => Ok(Self::NoStore),
            other => Err(Error::InvalidCacheTier(other.to_string())),
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    /// Spec §7.1 table — each row asserted exactly. Failing this means
    /// SPEC-001 changed and the constants here need to follow.
    #[test]
    fn fast_headers_match_spec() {
        assert_eq!(
            CacheTier::Fast.headers(),
            TierHeaders {
                s_maxage: 300,
                stale_while_revalidate: 60,
                stale_if_error: 1200
            }
        );
    }

    #[test]
    fn medium_headers_match_spec() {
        assert_eq!(
            CacheTier::Medium.headers(),
            TierHeaders {
                s_maxage: 600,
                stale_while_revalidate: 120,
                stale_if_error: 1800
            }
        );
    }

    #[test]
    fn slow_headers_match_spec() {
        assert_eq!(
            CacheTier::Slow.headers(),
            TierHeaders {
                s_maxage: 1800,
                stale_while_revalidate: 300,
                stale_if_error: 7200
            }
        );
    }

    #[test]
    fn slow_browser_headers_match_spec() {
        assert_eq!(
            CacheTier::SlowBrowser.headers(),
            TierHeaders {
                s_maxage: 900,
                stale_while_revalidate: 60,
                stale_if_error: 1800
            }
        );
    }

    #[test]
    fn static_headers_match_spec() {
        assert_eq!(
            CacheTier::Static.headers(),
            TierHeaders {
                s_maxage: 3600,
                stale_while_revalidate: 600,
                stale_if_error: 28800
            }
        );
    }

    #[test]
    fn daily_headers_match_spec() {
        assert_eq!(
            CacheTier::Daily.headers(),
            TierHeaders {
                s_maxage: 86400,
                stale_while_revalidate: 3600,
                stale_if_error: 172_800
            }
        );
    }

    #[test]
    fn no_store_emits_no_store_directive() {
        assert_eq!(CacheTier::NoStore.cache_control(), "no-store");
    }

    #[test]
    fn cache_control_format_for_fast() {
        assert_eq!(
            CacheTier::Fast.cache_control(),
            "public, s-maxage=300, stale-while-revalidate=60, stale-if-error=1200"
        );
    }

    #[test]
    fn from_str_round_trip_for_every_variant() {
        for tier in [
            CacheTier::Fast,
            CacheTier::Medium,
            CacheTier::Slow,
            CacheTier::SlowBrowser,
            CacheTier::Static,
            CacheTier::Daily,
            CacheTier::NoStore,
        ] {
            let s = tier.as_str();
            let parsed: CacheTier = s.parse().expect("parse");
            assert_eq!(parsed, tier);
        }
    }

    #[test]
    fn from_str_rejects_unknown() {
        let err = "turbo".parse::<CacheTier>().unwrap_err();
        assert!(matches!(err, Error::InvalidCacheTier(_)));
        assert!(err.to_string().contains("turbo"));
    }

    #[test]
    fn display_uses_kebab_case() {
        assert_eq!(format!("{}", CacheTier::SlowBrowser), "slow-browser");
        assert_eq!(format!("{}", CacheTier::NoStore), "no-store");
    }

    #[test]
    fn serde_uses_kebab_case() {
        let json = serde_json::to_string(&CacheTier::SlowBrowser).expect("serialize");
        assert_eq!(json, "\"slow-browser\"");
        let parsed: CacheTier = serde_json::from_str("\"daily\"").expect("parse");
        assert_eq!(parsed, CacheTier::Daily);
    }
}
