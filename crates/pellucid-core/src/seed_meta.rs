//! Seed metadata — the canonical companion to a cached envelope. Stored
//! in the `seed_meta` SQLite table (SPEC-001 §6.1) so health checks can
//! distinguish "missing" from "stale" without re-parsing the envelope.

use serde::{Deserialize, Serialize};

/// Discriminator for the data flowing through `seed_meta` and the
/// `state` slot of an [`crate::EnvelopeMeta`]. Mirrors the original
/// WorldMonitor `LoreDeepCodeReview.md §1.4` taxonomy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SeedState {
    /// Fresh data from the upstream within this seeder cycle.
    Live,
    /// Cached data older than the upstream's expected refresh cadence
    /// but younger than the cascade fallback threshold.
    Stale,
    /// Cascade fallback data emitted when the live source is down (e.g.
    /// theater-posture's "live → stale → backup" cascade group).
    Backup,
    /// Bootstrap-only data shipped with the deployment; no upstream call
    /// has yet succeeded since the deployment came online.
    Seeded,
}

/// A row in the `seed_meta` SQLite table. Every successful publish writes
/// one of these alongside the canonical envelope row in `kv_envelope`.
///
/// Field semantics map directly to the columns in
/// `crates/pellucid-db/migrations/0001_initial.sql` (which lands at T1.2).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SeedMeta {
    /// `fetched_at_ms` — Unix-epoch milliseconds of the upstream call
    /// that produced this envelope.
    pub fetched_at_ms: i64,
    /// `ttl_ms` — how long this envelope remains usable before
    /// transitioning to `Stale` for cascade-aware health checks.
    pub ttl_ms: i64,
    /// `last_run_id` — UUID of the seeder run that wrote this envelope.
    /// Used by atomic_publish (SPEC-001 §7.4) to recover from partial
    /// crash-during-publish states.
    pub last_run_id: String,
    /// Optional source version (commit sha, schema version) for
    /// cross-deployment compatibility checks.
    pub source_version: Option<String>,
    /// Number of rows in the data payload, surfaced verbatim for the
    /// /api/health endpoint's "STALE_SEED" classifier.
    pub record_count: Option<i64>,
    /// Cascade group label so /api/health can tolerate single-slot
    /// misses inside a group (live/stale/backup).
    pub cascade_group: Option<String>,
}

impl SeedMeta {
    /// True if `fetched_at_ms + ttl_ms` is in the past relative to `now`.
    #[must_use]
    pub const fn is_expired_at(&self, now_ms: i64) -> bool {
        self.fetched_at_ms.saturating_add(self.ttl_ms) <= now_ms
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn fixture() -> SeedMeta {
        SeedMeta {
            fetched_at_ms: 1_700_000_000_000,
            ttl_ms: 60_000,
            last_run_id: "01234567-89ab-cdef-0123-456789abcdef".into(),
            source_version: Some("aviationstack:v1".into()),
            record_count: Some(42),
            cascade_group: Some("aviation-flights".into()),
        }
    }

    #[test]
    fn round_trip_through_serde_preserves_every_field() {
        let original = fixture();
        let json = serde_json::to_string(&original).expect("serialize");
        let parsed: SeedMeta = serde_json::from_str(&json).expect("parse");
        assert_eq!(original, parsed);
    }

    #[test]
    fn deserialize_omits_optional_fields() {
        let json = r#"{
            "fetched_at_ms": 1700000000000,
            "ttl_ms": 60000,
            "last_run_id": "abc"
        }"#;
        let parsed: SeedMeta = serde_json::from_str(json).expect("parse");
        assert!(parsed.source_version.is_none());
        assert!(parsed.record_count.is_none());
        assert!(parsed.cascade_group.is_none());
    }

    #[test]
    fn is_expired_at_detects_just_after_ttl() {
        let m = fixture();
        let just_inside = m.fetched_at_ms + m.ttl_ms - 1;
        let exactly_at = m.fetched_at_ms + m.ttl_ms;
        let after = m.fetched_at_ms + m.ttl_ms + 1;
        assert!(!m.is_expired_at(just_inside));
        assert!(m.is_expired_at(exactly_at));
        assert!(m.is_expired_at(after));
    }

    #[test]
    fn is_expired_at_handles_overflow_via_saturating_add() {
        let m = SeedMeta {
            fetched_at_ms: i64::MAX - 1,
            ttl_ms: 100,
            last_run_id: "x".into(),
            source_version: None,
            record_count: None,
            cascade_group: None,
        };
        // saturating_add caps at i64::MAX; only `now_ms == i64::MAX`
        // satisfies the `<=` predicate.
        assert!(!m.is_expired_at(i64::MAX - 5));
        assert!(m.is_expired_at(i64::MAX));
    }

    #[test]
    fn seed_state_serde_uses_kebab_case() {
        assert_eq!(
            serde_json::to_string(&SeedState::Live).expect("ser"),
            "\"live\""
        );
        assert_eq!(
            serde_json::from_str::<SeedState>("\"backup\"").expect("parse"),
            SeedState::Backup
        );
    }

    #[test]
    fn seed_state_round_trip_for_every_variant() {
        for state in [
            SeedState::Live,
            SeedState::Stale,
            SeedState::Backup,
            SeedState::Seeded,
        ] {
            let s = serde_json::to_string(&state).expect("ser");
            let parsed: SeedState = serde_json::from_str(&s).expect("parse");
            assert_eq!(state, parsed);
        }
    }
}
