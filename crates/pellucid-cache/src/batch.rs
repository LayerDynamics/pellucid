//! Batch reads — single-transaction multi-key fetch used by
//! `/api/bootstrap` (SPEC-001 §3.3 OP-4) and any handler that needs to
//! hydrate many cache slots in one round-trip.

use std::collections::HashMap;

use serde::de::DeserializeOwned;
use sqlx::Row;

use pellucid_db::Pool;

/// Per-key outcome inside a [`get_cached_json_batch`] result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BatchHit<T> {
    /// Live value within TTL.
    Fresh(T),
    /// Value stored but TTL expired.
    Stale(T),
    /// Negative sentinel still in TTL.
    NegativeSentinel,
    /// Key not present in cache.
    Miss,
}

/// Read N keys in a single transaction, returning a map keyed on the
/// requested cache_key. Missing keys map to [`BatchHit::Miss`].
///
/// # Errors
/// Returns [`sqlx::Error`] when the transactional SELECT fails, or
/// [`sqlx::Error::Decode`] if any stored payload fails JSON parse.
pub async fn get_cached_json_batch<T: DeserializeOwned>(
    pool: &Pool,
    keys: &[&str],
) -> Result<HashMap<String, BatchHit<T>>, sqlx::Error> {
    if keys.is_empty() {
        return Ok(HashMap::new());
    }

    let mut tx = pool.begin().await?;
    let now = pellucid_core::now_ms();
    let mut out: HashMap<String, BatchHit<T>> = HashMap::with_capacity(keys.len());

    // Seed every requested key with Miss; rows we find overwrite below.
    for k in keys {
        out.insert((*k).to_string(), BatchHit::Miss);
    }

    // Build IN (?, ?, ...) with one placeholder per key.
    let placeholders = (1..=keys.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let sql = format!(
        "SELECT cache_key, payload, fetched_at_ms, ttl_ms, is_negative
         FROM kv_envelope WHERE cache_key IN ({placeholders})"
    );
    let mut q = sqlx::query(&sql);
    for k in keys {
        q = q.bind(*k);
    }
    let rows = q.fetch_all(&mut *tx).await?;
    tx.commit().await?;

    for row in rows {
        let key: String = row.get(0);
        let is_negative: i64 = row.get(4);
        let fetched: i64 = row.get(2);
        let ttl: i64 = row.get(3);
        let alive = fetched.saturating_add(ttl) > now;

        if is_negative != 0 {
            out.insert(
                key,
                if alive {
                    BatchHit::NegativeSentinel
                } else {
                    BatchHit::Miss
                },
            );
            continue;
        }

        let payload: String = row.get(1);
        let value: T = serde_json::from_str(&payload).map_err(|e| {
            sqlx::Error::Decode(Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
        })?;
        out.insert(
            key,
            if alive {
                BatchHit::Fresh(value)
            } else {
                BatchHit::Stale(value)
            },
        );
    }

    Ok(out)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_core::Envelope;
    use pellucid_db::open_in_memory;

    use crate::kv::{set_cached_json, set_negative_sentinel};

    #[tokio::test]
    async fn empty_keys_returns_empty_map() {
        let pool = open_in_memory().await.expect("pool");
        let out: HashMap<String, BatchHit<serde_json::Value>> =
            get_cached_json_batch(&pool, &[]).await.expect("batch");
        assert!(out.is_empty());
    }

    #[tokio::test]
    async fn batch_returns_fresh_stale_negative_miss() {
        let pool = open_in_memory().await.expect("pool");
        // fresh
        set_cached_json(
            &pool,
            "fresh",
            &Envelope::new(serde_json::json!({"a": 1})),
            60_000,
        )
        .await
        .expect("set fresh");
        // stale
        set_cached_json(
            &pool,
            "stale",
            &Envelope::new(serde_json::json!({"b": 2})),
            0,
        )
        .await
        .expect("set stale");
        // negative sentinel
        set_negative_sentinel(&pool, "neg", 60_000)
            .await
            .expect("neg");
        // no `miss` row at all — request must classify it as Miss.

        let out: HashMap<String, BatchHit<serde_json::Value>> =
            get_cached_json_batch(&pool, &["fresh", "stale", "neg", "miss"])
                .await
                .expect("batch");

        assert!(matches!(out.get("fresh").unwrap(), BatchHit::Fresh(_)));
        assert!(matches!(out.get("stale").unwrap(), BatchHit::Stale(_)));
        assert_eq!(out.get("neg").unwrap(), &BatchHit::NegativeSentinel);
        assert_eq!(out.get("miss").unwrap(), &BatchHit::Miss);
    }

    #[tokio::test]
    async fn batch_runs_in_single_transaction() {
        // 67-key fast tier from spec §3.3 OP-4 — pre-allocate a stand-in
        // sized similarly to verify the IN-clause builder handles many keys.
        let pool = open_in_memory().await.expect("pool");
        let keys: Vec<String> = (0..67).map(|i| format!("bootstrap:k{i}")).collect();
        let key_refs: Vec<&str> = keys.iter().map(String::as_str).collect();

        // Populate ~half the keys.
        for k in keys.iter().take(33) {
            set_cached_json(&pool, k, &Envelope::new(serde_json::json!({})), 60_000)
                .await
                .expect("set");
        }

        let out: HashMap<String, BatchHit<serde_json::Value>> =
            get_cached_json_batch(&pool, &key_refs)
                .await
                .expect("batch");

        assert_eq!(out.len(), 67);
        let fresh_count = out
            .values()
            .filter(|v| matches!(v, BatchHit::Fresh(_)))
            .count();
        let miss_count = out.values().filter(|v| matches!(v, BatchHit::Miss)).count();
        assert_eq!(fresh_count, 33);
        assert_eq!(miss_count, 34);
    }

    #[tokio::test]
    async fn negative_sentinel_decays_to_miss_in_batch() {
        let pool = open_in_memory().await.expect("pool");
        set_negative_sentinel(&pool, "neg-decay", 0)
            .await
            .expect("neg");
        let out: HashMap<String, BatchHit<serde_json::Value>> =
            get_cached_json_batch(&pool, &["neg-decay"])
                .await
                .expect("batch");
        assert_eq!(out.get("neg-decay").unwrap(), &BatchHit::Miss);
    }
}
