//! Static endpoint → required-tier map (SPEC-001 §14.2).
//!
//! Direct port of the original WorldMonitor `ENDPOINT_ENTITLEMENTS`
//! constant. The table is a `&'static [(&str, u8)]` so the lookup is
//! a compile-time-resolved linear scan over four entries — faster
//! than a hashed `phf::Map` at this size and one fewer dep.
//! The legacy `PREMIUM_RPC_PATHS` array (33 paths) was retired per
//! SPEC-001 §14.4 (H4 fix); any path not present here defaults to
//! [`Tier::Anonymous`].

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
pub const ENDPOINT_ENTITLEMENTS: &[(&str, u8)] = &[
    ("/api/market/v1/analyze-stock", 3),
    ("/api/market/v1/get-stock-analysis-history", 3),
    ("/api/market/v1/backtest-stock", 3),
    ("/api/market/v1/list-stored-stock-backtests", 3),
];

/// Required tier for `path`. Defaults to [`Tier::Anonymous`] for any
/// path not present in [`ENDPOINT_ENTITLEMENTS`].
#[must_use]
pub fn tier_for_path(path: &str) -> Tier {
    for (p, rank) in ENDPOINT_ENTITLEMENTS {
        if *p == path {
            return Tier::from_rank(*rank);
        }
    }
    Tier::Anonymous
}

/// Number of paths requiring authentication. Used by SPEC-001 §14.4
/// regression tests so the count cannot drift silently.
#[must_use]
pub const fn premium_path_count() -> usize {
    ENDPOINT_ENTITLEMENTS.len()
}

/// Iterator over `(path, required_tier)` for diagnostic dumps.
pub fn iter_premium_paths() -> impl Iterator<Item = (&'static str, Tier)> {
    ENDPOINT_ENTITLEMENTS
        .iter()
        .map(|(path, rank)| (*path, Tier::from_rank(*rank)))
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
    fn iter_premium_paths_lists_every_entry() {
        let collected: Vec<_> = iter_premium_paths().collect();
        assert_eq!(collected.len(), 4);
        assert!(collected
            .iter()
            .all(|(_, tier)| *tier == Tier::Tier2));
    }

    #[test]
    fn endpoint_entitlements_has_no_duplicates() {
        // A linear-scan table is correct only if entries are unique.
        let mut paths: Vec<&str> = ENDPOINT_ENTITLEMENTS.iter().map(|(p, _)| *p).collect();
        paths.sort_unstable();
        let len_before = paths.len();
        paths.dedup();
        assert_eq!(
            len_before,
            paths.len(),
            "ENDPOINT_ENTITLEMENTS must not contain duplicate paths"
        );
    }
}
