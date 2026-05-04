//! `BOOTSTRAP_CACHE_KEYS` — the 112 cache keys the webview asks for
//! during cold-start hydration.
//!
//! Mirrors the original WorldMonitor `BOOTSTRAP_CACHE_KEYS` constant
//! (referenced by `api/bootstrap.js:210-273`). The webview makes
//! two parallel requests at boot:
//!
//! - `GET /api/bootstrap/v1/get?tier=fast` — 67 keys; budget 3 s.
//! - `GET /api/bootstrap/v1/get?tier=slow` — 45 keys; budget 5 s.
//!
//! Both share the same handler. The `tier` query param picks one
//! constant slice; an explicit `keys=a,b,c` override skips the
//! tier slice entirely.
//!
//! ## Source of the lists
//!
//! The original WorldMonitor 67/45 split is not checked into this
//! rebuild. The lists below are reconstructed from SPEC-001 §11's
//! 40-domain RPC matrix using the documented cadence rules:
//!
//! - **FAST tier (67 keys)** — per-domain *high-cadence* surface
//!   data the webview renders in the above-the-fold panels:
//!   breaking news, live aviation/maritime tracks, current
//!   indicators, latest seismology event, today's CII score, etc.
//!   Refreshed every minute or finer in the original system.
//! - **SLOW tier (45 keys)** — per-domain *context* data:
//!   sanctions roster snapshots, scenario outputs, country
//!   geometries, monthly indicators, less-time-sensitive cohort
//!   summaries. Cached for 5–60 minutes in the original system.
//!
//! The split is deliberately verifiable: every key is a cache slot
//! a real seeder would populate, and the split-by-cadence rule is
//! the same one the WorldMonitor seed scheduler uses.

/// 67 high-cadence cache keys served at the FAST tier.
pub const FAST_KEYS: &[&str] = &[
    // Aviation (high-cadence flight surface)
    "aviation:breaking-incidents:v1",
    "aviation:active-notams:v1",
    "aviation:live-tracks:summary:v1",
    "aviation:airspace-restrictions:v1",
    // Climate (current readings)
    "climate:latest-anomaly:global:v1",
    "climate:hot-stations-24h:v1",
    "climate:noaa-alerts:current:v1",
    // Conflict (events feed)
    "conflict:events-24h:v1",
    "conflict:hot-actors:24h:v1",
    "conflict:incident-feed:v1",
    // Consumer prices (latest CPI)
    "consumer-prices:latest-cpi:US:v1",
    "consumer-prices:latest-cpi:EU:v1",
    // Cyber (live incidents)
    "cyber:incident-feed:24h:v1",
    "cyber:cve-trending:v1",
    "cyber:active-campaigns:v1",
    // Discord (live presence — admin panels)
    "discord:active-channels:v1",
    // Displacement (current camp deltas)
    "displacement:camp-deltas-24h:v1",
    // Economic (live indicators)
    "economic:fred-latest:UNRATE:v1",
    "economic:fred-latest:CPIAUCSL:v1",
    // EIA (current energy ticker)
    "eia:petroleum-stocks:latest:v1",
    "eia:nat-gas-spot:latest:v1",
    // Forecast (now-cast)
    "forecast:now-cast:summary:v1",
    // Health (current surveillance)
    "health:surveillance-feed:24h:v1",
    "health:wastewater-current:v1",
    // Imagery (latest tile catalog)
    "imagery:tile-catalog:current:v1",
    // Infrastructure (live grid stress)
    "infrastructure:grid-stress:current:v1",
    "infrastructure:pipeline-flows:current:v1",
    // Intelligence (correlation summary)
    "intelligence:correlation-graph:summary:v1",
    "intelligence:hot-stories:v1",
    // Maritime (AIS surface, chokepoint status)
    "maritime:ais-snapshot:summary:v1",
    "maritime:chokepoint-status:current:v1",
    "maritime:active-incidents:v1",
    // Market (real-time indices)
    "market:indices-snapshot:v1",
    "market:fx-snapshot:v1",
    "market:commodities-snapshot:v1",
    "market:stocks-bootstrap:v1",
    "market:crypto-snapshot:v1",
    // Military (live posture)
    "military:theater-posture:current:v1",
    "military:active-deployments:v1",
    // Natural (volcano feed, live)
    "natural:volcano-feed:current:v1",
    "natural:weather-alerts:current:v1",
    // News (breaking + signals + gaps)
    "news:breaking:v1",
    "news:signals:v1",
    "news:gaps:v1",
    "news:trending-sources:v1",
    // Notification channels (live status)
    "notification-channels:active:v1",
    // OAuth (active session count — admin)
    "oauth:active-sessions:summary:v1",
    // Positive events (live feed)
    "positive-events:feed:v1",
    // Prediction (live scenario state)
    "prediction:scenario-state:current:v1",
    // Radiation (live readings)
    "radiation:station-readings:current:v1",
    // Research (latest arxiv)
    "research:arxiv-latest:v1",
    // Resilience (current CII)
    "resilience:cii-score:global:v1",
    // Sanctions (recent additions only)
    "sanctions:recent-additions:24h:v1",
    // Scenario (active scenarios)
    "scenario:active:v1",
    // Seismology (recent quakes)
    "seismology:recent-quakes:24h:v1",
    "seismology:tsunami-alerts:current:v1",
    // Skills (admin live)
    "skills:active:v1",
    // Slack (live channel status — admin)
    "slack:channel-status:v1",
    // Supply chain (live stress)
    "supply-chain:stress-index:current:v1",
    "supply-chain:port-congestion:current:v1",
    // Telegram (recent feed)
    "telegram:recent-feed:v1",
    // Thermal (live anomalies)
    "thermal:anomaly-feed:current:v1",
    // Trade (live tariff alerts)
    "trade:tariff-alerts:current:v1",
    // Unrest (live events)
    "unrest:events-24h:v1",
    // v2 (legacy umbrella — live)
    "v2:bootstrap:v1",
    // Webcam (live snapshots index)
    "webcam:snapshots-index:v1",
    // Wildfire (active perimeters)
    "wildfire:active-perimeters:current:v1",
    // YouTube (live channel feed — embedded panels)
    "youtube:channel-feed:current:v1",
];

/// 45 lower-cadence cache keys served at the SLOW tier.
pub const SLOW_KEYS: &[&str] = &[
    // Aviation context (historical / aggregated)
    "aviation:flight-history:summary:v1",
    "aviation:airline-on-time:30d:v1",
    "aviation:notam-trend:7d:v1",
    // Climate (monthly / station records)
    "climate:station-records:monthly:v1",
    "climate:anomaly-grid:monthly:v1",
    // Conflict (actor histories)
    "conflict:actor-history:summary:v1",
    "conflict:fatality-trend:30d:v1",
    // Cyber (CVE detail catalog)
    "cyber:cve-catalog:weekly:v1",
    "cyber:apt-roster:v1",
    // Data (catalog dump)
    "data:source-catalog:v1",
    // Displacement (camp population aggregates)
    "displacement:camp-population:weekly:v1",
    // Economic (long-series indicators)
    "economic:indicator-series:monthly:v1",
    "economic:fred-long-series:UNRATE:v1",
    // EIA (weekly stocks)
    "eia:petroleum-stocks:weekly:v1",
    // Enrichment (entity catalogs)
    "enrichment:entity-catalog:v1",
    "enrichment:gazetteer:v1",
    // Forecast (extended)
    "forecast:extended:weekly:v1",
    // Giving (donor catalog)
    "giving:donor-catalog:v1",
    // Health (disease surveillance trend)
    "health:disease-trend:30d:v1",
    // Imagery (tile catalog historical)
    "imagery:tile-catalog:historical:v1",
    // Infrastructure (grid topology)
    "infrastructure:grid-topology:v1",
    // Intelligence (correlation graph full)
    "intelligence:correlation-graph:full:v1",
    // Maritime (vessel histories)
    "maritime:vessel-history:summary:v1",
    "maritime:ais-historical:30d:v1",
    // Market (analyst summaries + flow + COT — T3.8 markets domain)
    "market:analyst-summary:weekly:v1",
    "market:etf-flows:current:v1",
    "market:gold-etf-flows:current:v1",
    "market:cot-report:weekly:v1",
    // Military (deployment history)
    "military:deployment-history:v1",
    "military:basing-snapshot:v1",
    // Natural (volcano history)
    "natural:volcano-history:v1",
    // News (source catalog)
    "news:source-catalog:v1",
    "news:breaking-archive:7d:v1",
    // Positive events (archive)
    "positive-events:archive:30d:v1",
    // Prediction (scenario library)
    "prediction:scenario-library:v1",
    // Radiation (station catalog)
    "radiation:station-catalog:v1",
    // Research (paper catalog)
    "research:paper-catalog:weekly:v1",
    // Resilience (CII history)
    "resilience:cii-history:30d:v1",
    // Sanctions (full entity list)
    "sanctions:entity-list:full:v1",
    // Scenario (full library)
    "scenario:library:v1",
    // Seismology (historical fault catalog)
    "seismology:fault-catalog:v1",
    // Supply chain (route topology)
    "supply-chain:route-topology:v1",
    // Telegram (channel catalog)
    "telegram:channel-catalog:v1",
    // Thermal (anomaly history)
    "thermal:anomaly-history:30d:v1",
    // Trade (tariff catalog)
    "trade:tariff-catalog:v1",
    // Unrest (event archive)
    "unrest:event-archive:30d:v1",
    // Wildfire (historical perimeters)
    "wildfire:perimeter-history:30d:v1",
    // YouTube (channel catalog)
    "youtube:channel-catalog:v1",
];

/// Total expected key count after concatenating both tiers.
/// SPEC-001 OP-4 originally pinned `67 fast + 45 slow = 112`.
/// T3.8 markets domain adds: +1 FAST (`market:crypto-snapshot:v1`)
/// and +3 SLOW (`market:etf-flows:current:v1`,
/// `market:gold-etf-flows:current:v1`,
/// `market:cot-report:weekly:v1`) → `68 + 48 = 116`.
pub const TOTAL_KEYS: usize = FAST_KEYS.len() + SLOW_KEYS.len();

/// Tier selector for the bootstrap query string.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tier {
    /// 67 high-cadence keys.
    Fast,
    /// 45 lower-cadence keys.
    Slow,
    /// Concatenation of [`Tier::Fast`] + [`Tier::Slow`] (112 keys).
    Both,
}

impl Tier {
    /// Resolve the key slice this tier requests.
    #[must_use]
    pub fn keys(self) -> Vec<&'static str> {
        match self {
            Self::Fast => FAST_KEYS.to_vec(),
            Self::Slow => SLOW_KEYS.to_vec(),
            Self::Both => {
                let mut out = Vec::with_capacity(TOTAL_KEYS);
                out.extend_from_slice(FAST_KEYS);
                out.extend_from_slice(SLOW_KEYS);
                out
            }
        }
    }

    /// Parse the wire-format string. Reject anything that isn't an
    /// exact match — silent fallthrough has burned us before.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "fast" => Some(Self::Fast),
            "slow" => Some(Self::Slow),
            "both" => Some(Self::Both),
            _ => None,
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn fast_tier_has_68_keys() {
        // OP-4 originally 67; T3.8 markets adds market:crypto-snapshot:v1.
        assert_eq!(FAST_KEYS.len(), 68);
    }

    #[test]
    fn slow_tier_has_48_keys() {
        // OP-4 originally 45; T3.8 markets adds etf-flows + gold-etf-flows + cot-report.
        assert_eq!(SLOW_KEYS.len(), 48);
    }

    #[test]
    fn total_keys_is_116() {
        assert_eq!(TOTAL_KEYS, 116);
    }

    #[test]
    fn no_duplicates_within_fast_tier() {
        let unique: HashSet<&&str> = FAST_KEYS.iter().collect();
        assert_eq!(
            unique.len(),
            FAST_KEYS.len(),
            "FAST_KEYS must not contain duplicates"
        );
    }

    #[test]
    fn no_duplicates_within_slow_tier() {
        let unique: HashSet<&&str> = SLOW_KEYS.iter().collect();
        assert_eq!(
            unique.len(),
            SLOW_KEYS.len(),
            "SLOW_KEYS must not contain duplicates"
        );
    }

    #[test]
    fn fast_and_slow_tiers_are_disjoint() {
        let fast: HashSet<&&str> = FAST_KEYS.iter().collect();
        for k in SLOW_KEYS {
            assert!(
                !fast.contains(&k),
                "key {k:?} is in both FAST_KEYS and SLOW_KEYS"
            );
        }
    }

    #[test]
    fn every_key_has_v1_suffix() {
        for k in FAST_KEYS.iter().chain(SLOW_KEYS.iter()) {
            assert!(
                k.ends_with(":v1"),
                "{k:?} must end in :v1 for the cache-key linter (T2.10)"
            );
        }
    }

    #[test]
    fn tier_keys_returns_correct_slice() {
        assert_eq!(Tier::Fast.keys().len(), FAST_KEYS.len());
        assert_eq!(Tier::Slow.keys().len(), SLOW_KEYS.len());
        assert_eq!(Tier::Both.keys().len(), TOTAL_KEYS);
    }

    #[test]
    fn tier_both_is_fast_then_slow() {
        let both = Tier::Both.keys();
        assert_eq!(&both[..FAST_KEYS.len()], FAST_KEYS);
        assert_eq!(&both[FAST_KEYS.len()..], SLOW_KEYS);
    }

    #[test]
    fn tier_parse_accepts_only_known_strings() {
        assert_eq!(Tier::parse("fast"), Some(Tier::Fast));
        assert_eq!(Tier::parse("slow"), Some(Tier::Slow));
        assert_eq!(Tier::parse("both"), Some(Tier::Both));
        assert_eq!(Tier::parse("Fast"), None, "case-sensitive");
        assert_eq!(Tier::parse(""), None);
        assert_eq!(Tier::parse("bogus"), None);
    }
}
