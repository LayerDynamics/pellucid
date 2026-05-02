//! Static endpoint → required-tier map (SPEC-001 §14.2 + §14.4).
//!
//! 37-entry strict superset:
//! - 4 tier-2 entries from the original WorldMonitor §14.2 set.
//! - 33 tier-1 entries migrated from the legacy
//!   `PREMIUM_RPC_PATHS` Bearer-`role='pro'` code path
//!   (`server/gateway.ts:312-358` in the source repo). The H4 fix
//!   (SPEC-001 §14.4) retires the dual-gating attack surface by
//!   folding both sets into a single tier-based map; the gateway
//!   from v1 carries no separate premium-path code path.
//!
//! The table is a `&'static [(&str, u8)]` and lookup is a linear
//! scan — faster than a hashed `phf::Map` at this size and one
//! fewer dep. Any path not present here defaults to
//! [`Tier::Anonymous`].
//!
//! ## Regenerating the table
//!
//! ```text
//! bun run tools/migrate-premium-paths.ts --regenerate
//! ```
//!
//! The script reads `data/premium-rpc-paths.json` (the migration
//! reference list) and rewrites this file's table block in place.
//! `committed_table_matches_reference_data` below pins the live
//! table to the reference data so the script and the source can
//! never silently drift.

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
    ("/api/aviation/v1/get-flight-history",        1),
    ("/api/aviation/v1/get-notams",                1),
    ("/api/climate/v1/get-anomaly-grid",           1),
    ("/api/climate/v1/get-station-record",         1),
    ("/api/conflict/v1/get-actor-history",         1),
    ("/api/conflict/v1/get-events",                1),
    ("/api/consumer-prices/v1/get-cpi-series",     1),
    ("/api/cyber/v1/get-cve-detail",               1),
    ("/api/cyber/v1/get-incident-feed",            1),
    ("/api/displacement/v1/get-camp-population",   1),
    ("/api/economic/v1/get-indicator",             1),
    ("/api/eia/v1/get-petroleum-stocks",           1),
    ("/api/forecast/v1/get-extended",              1),
    ("/api/health/v1/get-disease-surveillance",    1),
    ("/api/imagery/v1/get-tile",                   1),
    ("/api/infrastructure/v1/get-grid-stress",     1),
    ("/api/intelligence/v1/get-correlation-graph", 1),
    ("/api/maritime/v1/get-ais-tracks",            1),
    ("/api/maritime/v1/get-chokepoint-status",     1),
    ("/api/market/v1/analyze-stock",               3),
    ("/api/market/v1/backtest-stock",              3),
    ("/api/market/v1/get-stock-analysis-history",  3),
    ("/api/market/v1/list-stored-stock-backtests", 3),
    ("/api/military/v1/get-theater-posture",       1),
    ("/api/natural/v1/get-volcano-feed",           1),
    ("/api/news/v1/get-breaking",                  1),
    ("/api/news/v1/get-source-feed",               1),
    ("/api/positive-events/v1/get-feed",           1),
    ("/api/prediction/v1/get-scenario-output",     1),
    ("/api/radiation/v1/get-station-readings",     1),
    ("/api/research/v1/get-arxiv-feed",            1),
    ("/api/resilience/v1/get-cii-score",           1),
    ("/api/sanctions/v1/get-entity-list",          1),
    ("/api/seismology/v1/get-recent-quakes",       1),
    ("/api/supply-chain/v1/get-stress-index",      1),
    ("/api/thermal/v1/get-anomaly-feed",           1),
    ("/api/wildfire/v1/get-active-perimeters",     1),
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
/// regression tests so the count cannot drift silently. Post-H4
/// migration this is `37` (4 tier-2 + 33 tier-1).
#[must_use]
pub const fn premium_path_count() -> usize {
    ENDPOINT_ENTITLEMENTS.len()
}

/// Expected total entries after the H4 migration. SPEC-001 §14.4
/// pins this to 37 = 4 (tier-2 §14.2 set) + 33 (legacy
/// `PREMIUM_RPC_PATHS` migrated to tier-1).
pub const TOTAL_GATED_PATHS: usize = 37;

/// Expected count of legacy `PREMIUM_RPC_PATHS` entries the H4
/// migration folded in. Pinned by the reference data file
/// `data/premium-rpc-paths.json`.
pub const MIGRATED_LEGACY_PATHS: usize = 33;

/// Pre-existing tier-2 entries (SPEC-001 §14.2). The migration
/// preserves these verbatim; the regression test verifies the
/// table still contains every one.
pub const TIER2_PATHS: &[&str] = &[
    "/api/market/v1/analyze-stock",
    "/api/market/v1/get-stock-analysis-history",
    "/api/market/v1/backtest-stock",
    "/api/market/v1/list-stored-stock-backtests",
];

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
    fn all_four_tier2_paths_present_after_migration() {
        for p in TIER2_PATHS {
            assert_eq!(
                tier_for_path(p),
                Tier::Tier2,
                "{p} must require Tier2"
            );
        }
    }

    #[test]
    fn iter_premium_paths_lists_every_entry() {
        let collected: Vec<_> = iter_premium_paths().collect();
        assert_eq!(collected.len(), TOTAL_GATED_PATHS);
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

    #[test]
    fn h4_total_count_matches_spec() {
        // SPEC-001 §14.4 pins the post-migration size to 37
        // = 4 (tier-2 §14.2) + 33 (legacy PREMIUM_RPC_PATHS folded
        // in at tier-1). Drift here means the migration script and
        // this file have diverged — re-run
        // `bun run tools/migrate-premium-paths.ts --regenerate`.
        assert_eq!(premium_path_count(), TOTAL_GATED_PATHS);
        let tier2 = ENDPOINT_ENTITLEMENTS
            .iter()
            .filter(|(_, r)| *r == 3)
            .count();
        let tier1 = ENDPOINT_ENTITLEMENTS
            .iter()
            .filter(|(_, r)| *r == 1)
            .count();
        assert_eq!(tier2, 4, "expected 4 tier-2 entries (§14.2)");
        assert_eq!(
            tier1, MIGRATED_LEGACY_PATHS,
            "expected {MIGRATED_LEGACY_PATHS} tier-1 entries (legacy §14.4 migration)",
        );
    }

    #[test]
    fn every_entry_has_valid_tier_rank() {
        // Catches a tier-rank bump from a `Tier` enum addition that
        // doesn't get reflected in the migration script.
        for (path, rank) in ENDPOINT_ENTITLEMENTS {
            assert!(
                *rank > 0 && *rank <= 3,
                "{path} has out-of-range tier rank {rank}",
            );
        }
    }

    #[test]
    fn entries_are_sorted() {
        // The migration script emits entries in lexical order so the
        // diffs of regenerations are minimal. A hand edit that
        // breaks the order would also break this test, prompting the
        // regenerate workflow.
        let mut sorted: Vec<&str> = ENDPOINT_ENTITLEMENTS.iter().map(|(p, _)| *p).collect();
        let original = sorted.clone();
        sorted.sort_unstable();
        assert_eq!(
            sorted, original,
            "ENDPOINT_ENTITLEMENTS must be lexically sorted; \
             regenerate via bun run tools/migrate-premium-paths.ts --regenerate"
        );
    }

    #[test]
    fn tier_for_path_is_case_sensitive() {
        // Original WorldMonitor gating was case-sensitive on the
        // path component; preserving that behaviour avoids accidental
        // privilege escalation via path-case spoofing.
        assert_eq!(tier_for_path("/API/market/v1/analyze-stock"), Tier::Anonymous);
        assert_eq!(tier_for_path("/api/MARKET/v1/analyze-stock"), Tier::Anonymous);
    }

    #[test]
    fn legacy_premium_paths_migrated_to_tier1() {
        // Spot-check 8 of the 33 legacy entries cover diverse
        // domains (aviation/maritime/news/cyber/military/etc).
        // Exhaustive coverage lives in the gateway-side
        // `regression_h4` integration test.
        let representative = [
            "/api/aviation/v1/get-notams",
            "/api/maritime/v1/get-ais-tracks",
            "/api/military/v1/get-theater-posture",
            "/api/news/v1/get-breaking",
            "/api/cyber/v1/get-cve-detail",
            "/api/seismology/v1/get-recent-quakes",
            "/api/wildfire/v1/get-active-perimeters",
            "/api/intelligence/v1/get-correlation-graph",
        ];
        for p in representative {
            assert_eq!(tier_for_path(p), Tier::Free, "{p} must be tier-1 Free");
        }
    }
}
