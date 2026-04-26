//! Static endpoint → required-tier map (SPEC-001 §14.2).
//!
//! Direct port of the original WorldMonitor `ENDPOINT_ENTITLEMENTS`
//! constant. The `phf::Map` resolves at compile time so the lookup
//! is O(1) and zero-allocation. The legacy `PREMIUM_RPC_PATHS` array
//! (33 paths) was retired per SPEC-001 §14.4 (H4 fix); any path not
//! present here defaults to [`Tier::Anonymous`].

use phf::phf_map;

use pellucid_gateway::traits::Tier;

/// Compile-time path → numeric tier table.
///
/// Numeric encoding mirrors `Tier::rank()`:
/// - `0` = Anonymous
/// - `1` = Free
/// - `2` = Tier1
/// - `3` = Tier2
///
/// Only paths that require *more* than `Anonymous` are listed.
pub static ENDPOINT_ENTITLEMENTS: phf::Map<&'static str, u8> = phf_map! {
    "/api/market/v1/analyze-stock" => 3,
    "/api/market/v1/get-stock-analysis-history" => 3,
    "/api/market/v1/backtest-stock" => 3,
    "/api/market/v1/list-stored-stock-backtests" => 3,
};

/// Convert a numeric tier rank into the typed [`Tier`] enum.
#[must_use]
pub fn tier_from_rank(rank: u8) -> Tier {
    match rank {
        0 => Tier::Anonymous,
        1 => Tier::Free,
        2 => Tier::Tier1,
        _ => Tier::Tier2,
    }
}

/// Convert a [`Tier`] into its numeric rank.
#[must_use]
pub fn rank_for_tier(tier: Tier) -> u8 {
    tier.rank()
}

/// Required tier for `path`. Defaults to [`Tier::Anonymous`] for any
/// path not present in [`ENDPOINT_ENTITLEMENTS`].
#[must_use]
pub fn tier_for_path(path: &str) -> Tier {
    ENDPOINT_ENTITLEMENTS
        .get(path)
        .map(|r| tier_from_rank(*r))
        .unwrap_or(Tier::Anonymous)
}

/// Number of paths requiring authentication. Used by SPEC-001 §14.4
/// regression tests so the count cannot drift silently.
#[must_use]
pub fn premium_path_count() -> usize {
    ENDPOINT_ENTITLEMENTS.len()
}

/// Iterator over `(path, required_tier)` for diagnostic dumps.
pub fn iter_premium_paths() -> impl Iterator<Item = (&'static str, Tier)> {
    ENDPOINT_ENTITLEMENTS
        .entries()
        .map(|(path, rank)| (*path, tier_from_rank(*rank)))
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn unmapped_path_defaults_to_anonymous() {
        assert_eq!(tier_for_path("/api/anything"), Tier::Anonymous);
        assert_eq!(tier_for_path(""), Tier::Anonymous);
        assert_eq!(tier_for_path("/api/market/v1/get-flight-status"), Tier::Anonymous);
    }

    #[test]
    fn analyze_stock_is_tier2() {
        assert_eq!(tier_for_path("/api/market/v1/analyze-stock"), Tier::Tier2);
    }

    #[test]
    fn all_four_premium_paths_present() {
        let want = [
            "/api/market/v1/analyze-stock",
            "/api/market/v1/get-stock-analysis-history",
            "/api/market/v1/backtest-stock",
            "/api/market/v1/list-stored-stock-backtests",
        ];
        for p in want {
            assert_eq!(
                tier_for_path(p),
                Tier::Tier2,
                "{p} must require Tier2"
            );
        }
        assert_eq!(premium_path_count(), 4);
    }

    #[test]
    fn rank_round_trips_through_tier() {
        for tier in [Tier::Anonymous, Tier::Free, Tier::Tier1, Tier::Tier2] {
            assert_eq!(tier_from_rank(rank_for_tier(tier)), tier);
        }
    }

    #[test]
    fn rank_above_three_clamps_to_tier2() {
        // Defensive: any cache row with a stale numeric tier higher
        // than Tier2 must clamp to Tier2 (the highest known tier)
        // rather than silently roll over to Anonymous.
        assert_eq!(tier_from_rank(255), Tier::Tier2);
        assert_eq!(tier_from_rank(4), Tier::Tier2);
    }

    #[test]
    fn iter_premium_paths_lists_every_entry() {
        let collected: Vec<_> = iter_premium_paths().collect();
        assert_eq!(collected.len(), 4);
        assert!(collected
            .iter()
            .all(|(_, tier)| *tier == Tier::Tier2));
    }
}
