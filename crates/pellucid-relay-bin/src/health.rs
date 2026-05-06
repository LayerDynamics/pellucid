//! `/health` endpoint — the **L3 fix** per SPEC-001 §17.5.
//!
//! The original WorldMonitor relay's `/health` returned a
//! hardcoded `200 OK`. The L3 incident report observed that an
//! all-stale cache during seeder downtime never page'd anyone,
//! because Fly's healthcheck saw 200 and never restarted the
//! relay. The fix: read the `seed_meta` table, compute one
//! status per `cascade_group`, and degrade the HTTP response
//! when too many groups are stale or missing.
//!
//! ## Semantics
//!
//! For each unique `cascade_group` in `seed_meta`:
//!
//! - `fresh` — `now_ms - fetched_at_ms <= ttl_ms`.
//! - `stale` — `fetched_at_ms` set but TTL elapsed.
//! - `missing` — no row at all (rare — indicates a seeder that
//!   never ran).
//!
//! Aggregate response status:
//!
//! - `200 OK` when every group is `fresh`.
//! - `200 OK` with a `degraded` body field when 1+ group is
//!   `stale` (still serving last-known data — the cascade will
//!   recover when the seeder next succeeds).
//! - `503 Service Unavailable` when the fraction of `missing`
//!   groups exceeds `outage_threshold` (default 50%).
//!
//! Fly / Railway healthchecks treat 503 as "restart this
//! machine"; the threshold is conservative enough that a
//! single domain outage doesn't trip a restart, but a
//! whole-cascade collapse does.

use std::collections::BTreeMap;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use sqlx::Row;

use pellucid_db::Pool;

/// HTTP path the healthcheck binds.
pub const HEALTH_PATH: &str = "/health";

/// Default outage threshold — 50% missing groups → 503.
pub const DEFAULT_OUTAGE_THRESHOLD: f64 = 0.5;

/// One cascade-group reading.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupStatus {
    /// Cascade group name (e.g. `"theater-posture"`).
    pub group: String,
    /// `fresh | stale | missing`.
    pub state: GroupState,
    /// Wall-clock ms when the freshest member of the group
    /// last published. `None` for `missing`.
    pub last_fetched_ms: Option<i64>,
    /// Member count in this group (number of `seed_meta` rows
    /// tagged `cascade_group = self.group`).
    pub member_count: i64,
}

/// State of one cascade group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GroupState {
    /// All members are within their TTL.
    Fresh,
    /// At least one member's TTL has elapsed.
    Stale,
    /// No `seed_meta` row exists for this group at all.
    Missing,
}

/// Top-level health response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HealthBody {
    /// Crate version.
    pub version: String,
    /// Wall-clock ms the cascade was computed.
    pub computed_at_ms: i64,
    /// `ok | degraded | outage`.
    pub status: HealthStatus,
    /// Per-group rows.
    pub groups: Vec<GroupStatus>,
    /// Counts: `{ fresh, stale, missing }`.
    pub summary: HealthSummary,
}

/// Top-level status flag.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    /// Every group is fresh.
    Ok,
    /// 1+ stale group, but missing fraction below threshold.
    Degraded,
    /// Missing fraction above threshold — Fly should restart.
    Outage,
}

/// Aggregate counts.
#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
pub struct HealthSummary {
    /// Number of fresh groups.
    pub fresh: usize,
    /// Number of stale groups.
    pub stale: usize,
    /// Number of missing groups.
    pub missing: usize,
}

/// Minimal app state the health route needs.
#[derive(Clone, Debug)]
pub struct HealthState {
    /// SQLite pool — owned by the relay binary.
    pub pool: Pool,
    /// Outage threshold (`missing / total` ≥ this triggers 503).
    pub outage_threshold: f64,
    /// Fixed list of cascade groups the relay expects to see.
    /// When a group is in this list but missing from
    /// `seed_meta`, it surfaces as `Missing`. The relay
    /// derives this list from the registered seeder set at
    /// boot.
    pub expected_groups: Vec<String>,
}

/// Build the `/health` router.
pub fn health_router(state: HealthState) -> Router {
    Router::new()
        .route(HEALTH_PATH, get(health_handler))
        .with_state(state)
}

/// Compute the cascade once.
///
/// # Errors
/// Returns the underlying `sqlx::Error` if the `seed_meta`
/// query fails.
pub async fn compute_cascade(
    pool: &Pool,
    expected_groups: &[String],
    outage_threshold: f64,
) -> Result<HealthBody, sqlx::Error> {
    let now_ms = pellucid_core::now_ms();
    let rows = sqlx::query(
        "SELECT cascade_group, MIN(fetched_at_ms) AS oldest_fetched, \
         MIN(fetched_at_ms + ttl_ms) AS earliest_expiry, \
         COUNT(*) AS member_count \
         FROM seed_meta \
         WHERE cascade_group IS NOT NULL AND cascade_group <> '' \
         GROUP BY cascade_group",
    )
    .fetch_all(pool)
    .await?;

    let mut by_group: BTreeMap<String, GroupStatus> = BTreeMap::new();
    for row in rows {
        let group: String = row.try_get("cascade_group")?;
        let oldest_fetched: i64 = row.try_get("oldest_fetched")?;
        let earliest_expiry: i64 = row.try_get("earliest_expiry")?;
        let member_count: i64 = row.try_get("member_count")?;
        let state = if now_ms <= earliest_expiry {
            GroupState::Fresh
        } else {
            GroupState::Stale
        };
        by_group.insert(
            group.clone(),
            GroupStatus {
                group,
                state,
                last_fetched_ms: Some(oldest_fetched),
                member_count,
            },
        );
    }

    // Pad in any expected groups missing from `seed_meta`.
    for expected in expected_groups {
        by_group.entry(expected.clone()).or_insert(GroupStatus {
            group: expected.clone(),
            state: GroupState::Missing,
            last_fetched_ms: None,
            member_count: 0,
        });
    }

    let groups: Vec<GroupStatus> = by_group.into_values().collect();
    let mut fresh = 0usize;
    let mut stale = 0usize;
    let mut missing = 0usize;
    for g in &groups {
        match g.state {
            GroupState::Fresh => fresh += 1,
            GroupState::Stale => stale += 1,
            GroupState::Missing => missing += 1,
        }
    }
    let total = groups.len().max(1);
    let missing_fraction = missing as f64 / total as f64;
    let status = if missing_fraction >= outage_threshold {
        HealthStatus::Outage
    } else if stale > 0 || missing > 0 {
        HealthStatus::Degraded
    } else {
        HealthStatus::Ok
    };

    Ok(HealthBody {
        version: env!("CARGO_PKG_VERSION").to_string(),
        computed_at_ms: now_ms,
        status,
        groups,
        summary: HealthSummary {
            fresh,
            stale,
            missing,
        },
    })
}

/// Map a cascade body to an HTTP status code.
#[must_use]
pub const fn status_code_for(status: HealthStatus) -> StatusCode {
    match status {
        HealthStatus::Ok | HealthStatus::Degraded => StatusCode::OK,
        HealthStatus::Outage => StatusCode::SERVICE_UNAVAILABLE,
    }
}

async fn health_handler(State(state): State<HealthState>) -> impl IntoResponse {
    match compute_cascade(&state.pool, &state.expected_groups, state.outage_threshold).await {
        Ok(body) => {
            let code = status_code_for(body.status);
            (code, Json(body)).into_response()
        }
        Err(e) => {
            let body = serde_json::json!({
                "status": "outage",
                "error": format!("seed_meta query failed: {e}"),
            });
            (StatusCode::SERVICE_UNAVAILABLE, Json(body)).into_response()
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use pellucid_db::open_in_memory;
    use tower::ServiceExt;

    async fn insert_seed_meta(
        pool: &Pool,
        cache_key: &str,
        group: &str,
        fetched_at_ms: i64,
        ttl_ms: i64,
    ) {
        sqlx::query(
            "INSERT INTO seed_meta \
             (cache_key, fetched_at_ms, ttl_ms, last_run_id, source_version, record_count, cascade_group) \
             VALUES (?, ?, ?, 'r-1', 'v1', 1, ?)",
        )
        .bind(cache_key)
        .bind(fetched_at_ms)
        .bind(ttl_ms)
        .bind(group)
        .execute(pool)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn cascade_all_fresh_yields_ok() {
        let pool = open_in_memory().await.unwrap();
        let now = pellucid_core::now_ms();
        // 60-second TTL, fetched 5s ago — fresh.
        insert_seed_meta(&pool, "k1", "alpha", now - 5_000, 60_000).await;
        insert_seed_meta(&pool, "k2", "beta", now - 2_000, 60_000).await;
        let body = compute_cascade(&pool, &["alpha".into(), "beta".into()], 0.5)
            .await
            .unwrap();
        assert_eq!(body.status, HealthStatus::Ok);
        assert_eq!(body.summary.fresh, 2);
        assert_eq!(body.summary.stale, 0);
        assert_eq!(body.summary.missing, 0);
    }

    #[tokio::test]
    async fn cascade_one_stale_yields_degraded_with_200() {
        let pool = open_in_memory().await.unwrap();
        let now = pellucid_core::now_ms();
        insert_seed_meta(&pool, "k1", "alpha", now - 90_000, 60_000).await; // stale
        insert_seed_meta(&pool, "k2", "beta", now - 5_000, 60_000).await; // fresh
        let body = compute_cascade(&pool, &["alpha".into(), "beta".into()], 0.5)
            .await
            .unwrap();
        assert_eq!(body.status, HealthStatus::Degraded);
        assert_eq!(status_code_for(body.status), StatusCode::OK);
    }

    #[tokio::test]
    async fn cascade_majority_missing_yields_outage_with_503() {
        let pool = open_in_memory().await.unwrap();
        // Two expected, one present.
        let now = pellucid_core::now_ms();
        insert_seed_meta(&pool, "k1", "alpha", now - 5_000, 60_000).await;
        let body = compute_cascade(&pool, &["alpha".into(), "beta".into()], 0.5)
            .await
            .unwrap();
        assert_eq!(body.status, HealthStatus::Outage);
        assert_eq!(
            status_code_for(body.status),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert_eq!(body.summary.missing, 1);
    }

    #[tokio::test]
    async fn cascade_unexpected_group_in_seed_meta_still_surfaces() {
        let pool = open_in_memory().await.unwrap();
        let now = pellucid_core::now_ms();
        insert_seed_meta(&pool, "k1", "surprise", now - 5_000, 60_000).await;
        let body = compute_cascade(&pool, &[], 0.5).await.unwrap();
        // Unexpected group is fresh — counts as fresh.
        assert_eq!(body.groups.len(), 1);
        assert_eq!(body.groups[0].group, "surprise");
        assert_eq!(body.summary.fresh, 1);
    }

    #[tokio::test]
    async fn cascade_skips_seed_meta_rows_with_null_cascade_group() {
        let pool = open_in_memory().await.unwrap();
        sqlx::query(
            "INSERT INTO seed_meta \
             (cache_key, fetched_at_ms, ttl_ms, last_run_id, source_version, record_count, cascade_group) \
             VALUES ('orphan', 1, 60000, 'r', 'v', 1, NULL)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let body = compute_cascade(&pool, &[], 0.5).await.unwrap();
        // Empty cascade — `total = max(1)` → fraction = 0 → Ok.
        assert_eq!(body.status, HealthStatus::Ok);
        assert_eq!(body.groups.len(), 0);
    }

    #[tokio::test]
    async fn handler_returns_503_when_outage_status() {
        let pool = open_in_memory().await.unwrap();
        // 0 of 1 expected → 100% missing → outage.
        let state = HealthState {
            pool,
            outage_threshold: 0.5,
            expected_groups: vec!["alpha".into()],
        };
        let app = health_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(HEALTH_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn handler_returns_200_when_all_fresh() {
        let pool = open_in_memory().await.unwrap();
        let now = pellucid_core::now_ms();
        insert_seed_meta(&pool, "k1", "alpha", now - 1_000, 60_000).await;
        let state = HealthState {
            pool,
            outage_threshold: 0.5,
            expected_groups: vec!["alpha".into()],
        };
        let app = health_router(state);
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(HEALTH_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.get("status").and_then(|v| v.as_str()), Some("ok"));
    }
}
