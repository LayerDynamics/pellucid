//! Direct KV API — read, write, delete a single envelope row in
//! `kv_envelope` (SPEC-001 §6.1). The stampede-protected fetch lives in
//! [`super::coalesce`]; this module is the underlying SQLite plumbing.

use serde::de::DeserializeOwned;
use serde::Serialize;
use sqlx::Row;

use pellucid_core::{Envelope, validate_envelope_size};
use pellucid_db::Pool;

use crate::negative::DEFAULT_NEGATIVE_TTL_MS;

/// Outcome of a single-key cache read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CacheHit<T> {
    /// Live envelope payload still inside its TTL.
    Fresh(T),
    /// Envelope exists but its TTL has expired. Caller decides whether
    /// to serve stale or trigger a refresh.
    Stale(T),
    /// A negative-cache sentinel is in place; upstream returned null
    /// recently and we should not re-issue the fetch yet.
    NegativeSentinel,
    /// Nothing cached for this key.
    Miss,
}

/// Convenience wrapper around a [`Pool`] exposing the cache surface as
/// methods. Most callers use the free functions in this module + the
/// stampede coalescer; [`KvCache`] is here for tests and high-level
/// orchestration where carrying a `Pool` reference is awkward.
#[derive(Debug, Clone)]
pub struct KvCache {
    pool: Pool,
}

impl KvCache {
    /// Construct a cache view over an existing [`Pool`].
    #[must_use]
    pub const fn new(pool: Pool) -> Self {
        Self { pool }
    }

    /// Borrow the underlying pool.
    #[must_use]
    pub const fn pool(&self) -> &Pool {
        &self.pool
    }

    /// Read a single envelope row.
    ///
    /// # Errors
    /// Returns the underlying [`sqlx::Error`] when the SELECT fails.
    pub async fn get<T: DeserializeOwned>(&self, key: &str) -> Result<CacheHit<T>, sqlx::Error> {
        get_cached_json(&self.pool, key).await
    }

    /// Write an envelope row with the given TTL. Validates the encoded
    /// payload against [`MAX_ENVELOPE_BYTES`].
    ///
    /// # Errors
    /// Returns [`sqlx::Error`] on failed write or `RowTooBig` (mapped to
    /// [`sqlx::Error::Protocol`]) when the encoded payload exceeds 5 MiB.
    pub async fn set<T: Serialize>(
        &self,
        key: &str,
        value: &Envelope<T>,
        ttl_ms: i64,
    ) -> Result<(), sqlx::Error> {
        set_cached_json(&self.pool, key, value, ttl_ms).await
    }

    /// Delete a row by key.
    ///
    /// # Errors
    /// Returns [`sqlx::Error`] on failed delete.
    pub async fn delete(&self, key: &str) -> Result<(), sqlx::Error> {
        sqlx::query("DELETE FROM kv_envelope WHERE cache_key = ?1")
            .bind(key)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Drop a negative sentinel for the given key with the supplied TTL
    /// (or the default 120s if `ttl_ms` is `None`).
    ///
    /// # Errors
    /// Returns [`sqlx::Error`] on failed write.
    pub async fn set_negative(&self, key: &str, ttl_ms: Option<i64>) -> Result<(), sqlx::Error> {
        set_negative_sentinel(&self.pool, key, ttl_ms.unwrap_or(DEFAULT_NEGATIVE_TTL_MS)).await
    }
}

/// Read a single envelope row, returning a [`CacheHit`].
///
/// # Errors
/// Returns [`sqlx::Error`] when the SELECT fails. JSON parse failures on
/// stored payloads are mapped to [`sqlx::Error::Decode`] so callers see a
/// single error type at this layer.
pub async fn get_cached_json<T: DeserializeOwned>(
    pool: &Pool,
    key: &str,
) -> Result<CacheHit<T>, sqlx::Error> {
    let row = sqlx::query(
        "SELECT payload, fetched_at_ms, ttl_ms, is_negative
         FROM kv_envelope WHERE cache_key = ?1",
    )
    .bind(key)
    .fetch_optional(pool)
    .await?;

    let Some(row) = row else {
        return Ok(CacheHit::Miss);
    };

    let is_negative: i64 = row.get(3);
    if is_negative != 0 {
        let fetched: i64 = row.get(1);
        let ttl: i64 = row.get(2);
        if fetched.saturating_add(ttl) > pellucid_core::now_ms() {
            return Ok(CacheHit::NegativeSentinel);
        }
        return Ok(CacheHit::Miss);
    }

    let payload: String = row.get(0);
    let value: T = serde_json::from_str(&payload)
        .map_err(|e| sqlx::Error::Decode(Box::new(e) as Box<dyn std::error::Error + Send + Sync>))?;

    let fetched: i64 = row.get(1);
    let ttl: i64 = row.get(2);
    if fetched.saturating_add(ttl) > pellucid_core::now_ms() {
        Ok(CacheHit::Fresh(value))
    } else {
        Ok(CacheHit::Stale(value))
    }
}

/// Write a positive envelope row. Validates encoded size against
/// [`MAX_ENVELOPE_BYTES`].
///
/// # Errors
/// Returns [`sqlx::Error::Protocol`] when the encoded payload exceeds
/// the 5 MiB cap (SPEC-001 §7.4); otherwise propagates sqlx errors.
pub async fn set_cached_json<T: Serialize>(
    pool: &Pool,
    key: &str,
    value: &Envelope<T>,
    ttl_ms: i64,
) -> Result<(), sqlx::Error> {
    let payload = serde_json::to_string(value)
        .map_err(|e| sqlx::Error::Encode(Box::new(e) as Box<dyn std::error::Error + Send + Sync>))?;

    if let Err(too_big) = validate_envelope_size(payload.as_bytes()) {
        return Err(sqlx::Error::Protocol(too_big.to_string()));
    }

    let now = pellucid_core::now_ms();
    sqlx::query(
        "INSERT INTO kv_envelope
            (cache_key, payload, fetched_at_ms, ttl_ms, record_count, source_version, state, is_negative)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 0)
         ON CONFLICT(cache_key) DO UPDATE SET
            payload = excluded.payload,
            fetched_at_ms = excluded.fetched_at_ms,
            ttl_ms = excluded.ttl_ms,
            record_count = excluded.record_count,
            source_version = excluded.source_version,
            state = excluded.state,
            is_negative = 0",
    )
    .bind(key)
    .bind(&payload)
    .bind(now)
    .bind(ttl_ms)
    .bind(value.seed.record_count)
    .bind(value.seed.source_version.as_deref())
    .bind(value.seed.state.as_deref())
    .execute(pool)
    .await?;
    Ok(())
}

/// Write a negative-cache sentinel row.
///
/// # Errors
/// Returns [`sqlx::Error`] on write failure.
pub async fn set_negative_sentinel(
    pool: &Pool,
    key: &str,
    ttl_ms: i64,
) -> Result<(), sqlx::Error> {
    let now = pellucid_core::now_ms();
    sqlx::query(
        "INSERT INTO kv_envelope
            (cache_key, payload, fetched_at_ms, ttl_ms, record_count, source_version, state, is_negative)
         VALUES (?1, '{}', ?2, ?3, NULL, NULL, NULL, 1)
         ON CONFLICT(cache_key) DO UPDATE SET
            payload = '{}',
            fetched_at_ms = excluded.fetched_at_ms,
            ttl_ms = excluded.ttl_ms,
            record_count = NULL,
            source_version = NULL,
            state = NULL,
            is_negative = 1",
    )
    .bind(key)
    .bind(now)
    .bind(ttl_ms)
    .execute(pool)
    .await?;
    Ok(())
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use pellucid_core::{Envelope, MAX_ENVELOPE_BYTES};
    use pellucid_db::open_in_memory;

    fn sample_envelope() -> Envelope<serde_json::Value> {
        Envelope::new(serde_json::json!({"k": "v"}))
            .with_record_count(1)
            .with_source_version("v1")
    }

    #[tokio::test]
    async fn miss_on_unknown_key() {
        let pool = open_in_memory().await.expect("pool");
        let hit: CacheHit<serde_json::Value> = get_cached_json(&pool, "absent").await.expect("get");
        assert_eq!(hit, CacheHit::Miss);
    }

    #[tokio::test]
    async fn fresh_after_set_within_ttl() {
        let pool = open_in_memory().await.expect("pool");
        set_cached_json(&pool, "k", &sample_envelope(), 60_000).await.expect("set");
        let hit: CacheHit<serde_json::Value> = get_cached_json(&pool, "k").await.expect("get");
        match hit {
            CacheHit::Fresh(v) => assert_eq!(v["data"]["k"], "v"),
            other => panic!("expected fresh, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn stale_after_zero_ttl() {
        let pool = open_in_memory().await.expect("pool");
        set_cached_json(&pool, "k", &sample_envelope(), 0).await.expect("set");
        // ttl=0 means now+0 == now, so the comparison fetched+ttl > now is false → stale.
        let hit: CacheHit<serde_json::Value> = get_cached_json(&pool, "k").await.expect("get");
        assert!(matches!(hit, CacheHit::Stale(_)));
    }

    #[tokio::test]
    async fn negative_sentinel_returns_negative_within_ttl() {
        let pool = open_in_memory().await.expect("pool");
        set_negative_sentinel(&pool, "k", 60_000).await.expect("neg");
        let hit: CacheHit<serde_json::Value> = get_cached_json(&pool, "k").await.expect("get");
        assert_eq!(hit, CacheHit::NegativeSentinel);
    }

    #[tokio::test]
    async fn negative_sentinel_decays_to_miss_after_ttl() {
        let pool = open_in_memory().await.expect("pool");
        set_negative_sentinel(&pool, "k", 0).await.expect("neg");
        let hit: CacheHit<serde_json::Value> = get_cached_json(&pool, "k").await.expect("get");
        assert_eq!(hit, CacheHit::Miss);
    }

    #[tokio::test]
    async fn delete_removes_row() {
        let pool = open_in_memory().await.expect("pool");
        let cache = KvCache::new(pool);
        cache.set("k", &sample_envelope(), 60_000).await.expect("set");
        cache.delete("k").await.expect("delete");
        let hit: CacheHit<serde_json::Value> = cache.get("k").await.expect("get");
        assert_eq!(hit, CacheHit::Miss);
    }

    #[tokio::test]
    async fn oversized_payload_is_rejected_by_set() {
        let pool = open_in_memory().await.expect("pool");
        // Build a payload guaranteed to exceed MAX_ENVELOPE_BYTES (5 MiB).
        let big = "x".repeat(MAX_ENVELOPE_BYTES);
        let env = Envelope::new(serde_json::json!({"big": big}));
        let err = set_cached_json(&pool, "k", &env, 60_000).await.unwrap_err();
        match err {
            sqlx::Error::Protocol(msg) => assert!(msg.contains("payload exceeds max size")),
            other => panic!("expected Protocol(too-big), got {other:?}"),
        }
    }

    #[tokio::test]
    async fn upsert_replaces_existing_row() {
        let pool = open_in_memory().await.expect("pool");
        set_cached_json(&pool, "k", &sample_envelope().with_record_count(1), 60_000)
            .await
            .expect("first");
        set_cached_json(&pool, "k", &sample_envelope().with_record_count(7), 60_000)
            .await
            .expect("second");
        let row = sqlx::query("SELECT record_count FROM kv_envelope WHERE cache_key = 'k'")
            .fetch_one(&pool)
            .await
            .expect("query");
        let n: i64 = row.get(0);
        assert_eq!(n, 7);
    }
}
