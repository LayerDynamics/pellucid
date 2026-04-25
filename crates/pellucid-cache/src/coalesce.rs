//! Stampede coalescing — concurrent reads of the same cache key share a
//! single upstream fetch.
//!
//! SPEC-001 §7.2 mandates that on a cold cache, N concurrent calls
//! produce **exactly one** upstream fetcher invocation and one cache
//! write. Implementation: a `DashMap` of `Arc<OnceCell<Result>>` keyed
//! on cache_key. The first caller that inserts an entry runs the
//! fetcher; everyone else awaits the OnceCell.
//!
//! After the fetcher resolves, the result is written back to SQLite
//! (positive envelope or negative sentinel) and the in-flight entry is
//! evicted so the next miss starts fresh.

use std::future::Future;
use std::sync::Arc;

use dashmap::DashMap;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tokio::sync::{Mutex, OnceCell};

use pellucid_core::{CacheTier, Envelope};
use pellucid_db::Pool;

use crate::kv::{CacheHit, get_cached_json, set_cached_json, set_negative_sentinel};
use crate::negative::DEFAULT_NEGATIVE_TTL_MS;

/// Result of a single coalesced fetch — what gets stored in the
/// in-flight OnceCell and broadcast to every awaiter for the same key.
#[derive(Debug, Clone)]
enum FetchOutcome<T: Clone> {
    /// Upstream returned a value. Written back to SQLite and returned.
    Hit(T),
    /// Upstream returned `None`. Negative sentinel written; subsequent
    /// awaiters and future reads see [`CacheHit::NegativeSentinel`] until
    /// the TTL elapses.
    Negative,
    /// Upstream errored. Stringified message is returned to every
    /// awaiter; the cache row is **not** written, so a retry is allowed
    /// after the in-flight entry is evicted.
    Error(String),
}

/// Tracks in-flight upstream fetches keyed on cache_key. Cloneable so
/// edge + sidecar can share one registry across handler invocations.
#[derive(Debug, Clone, Default)]
pub struct CoalesceRegistry {
    inflight: Arc<DashMap<String, Arc<OnceCell<FetchOutcomeStored>>>>,
}

/// Type-erased variant stored inside the OnceCell so the registry
/// doesn't need a generic parameter on the cell type itself.
type FetchOutcomeStored = FetchOutcome<serde_json::Value>;

impl CoalesceRegistry {
    /// Construct an empty registry.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the number of distinct keys currently in-flight. Used by
    /// integration tests to assert the registry's housekeeping behaviour.
    #[must_use]
    pub fn inflight_len(&self) -> usize {
        self.inflight.len()
    }
}

/// Stampede-protected fetch.
///
/// 1. Look up the row in `kv_envelope`. If present and fresh, return.
/// 2. If a negative sentinel is in place, return `None` without firing
///    the fetcher.
/// 3. Otherwise, register an in-flight cell for this key. The first
///    caller runs the fetcher; concurrent callers await the cell.
/// 4. On `Ok(Some(v))`: write the envelope row and return `Some(v)`.
/// 5. On `Ok(None)`: write a negative sentinel and return `None`.
/// 6. On `Err`: return the error to every awaiter and **do not** write
///    the cache. The in-flight entry is evicted in either case.
///
/// # Errors
/// Returns the [`sqlx::Error`] surfaced from the cache layer or, on
/// fetcher failure, [`sqlx::Error::Protocol`] wrapping the stringified
/// fetcher error.
pub async fn cached_fetch_json<T, F, Fut>(
    pool: &Pool,
    registry: &CoalesceRegistry,
    key: &str,
    tier: CacheTier,
    fetcher: F,
) -> Result<Option<T>, sqlx::Error>
where
    T: Serialize + DeserializeOwned + Clone + Send + Sync + 'static,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: Future<Output = Result<Option<T>, Box<dyn std::error::Error + Send + Sync>>>
        + Send
        + 'static,
{
    // 1. Cache check. Stored shape is the full Envelope<Value> wrapper, so
    //    we must pluck the `data` slot before decoding into T.
    match get_cached_json::<serde_json::Value>(pool, key).await? {
        CacheHit::Fresh(envelope) => return Ok(Some(decode_envelope_data::<T>(&envelope)?)),
        CacheHit::Stale(envelope) => {
            // Best-effort background refresh would go here; for the
            // foreground call we just return the stale value.
            return Ok(Some(decode_envelope_data::<T>(&envelope)?));
        }
        CacheHit::NegativeSentinel => return Ok(None),
        CacheHit::Miss => {}
    }

    // 2. Coalesced fetch.
    let cell = registry
        .inflight
        .entry(key.to_string())
        .or_insert_with(|| Arc::new(OnceCell::new()))
        .clone();

    // Wrap the fetcher in a Mutex so only the first awaiter that
    // calls `get_or_init` runs it. OnceCell already guarantees this,
    // but the Mutex makes it explicit when the fetcher itself needs
    // mutable state.
    let fetcher_holder = Arc::new(Mutex::new(Some(fetcher)));
    let pool_arc = pool.clone();
    let key_owned = key.to_string();
    let ttl_ms: i64 = i64::from(tier.headers().s_maxage) * 1000;

    let outcome = cell
        .get_or_init(|| async move {
            let f = match fetcher_holder.lock().await.take() {
                Some(f) => f,
                None => return FetchOutcome::Error("inflight fetcher already taken".to_string()),
            };
            match f().await {
                Ok(Some(value)) => match serde_json::to_value(&value) {
                    Ok(json) => {
                        let envelope = Envelope::new(json.clone());
                        match set_cached_json(&pool_arc, &key_owned, &envelope, ttl_ms).await {
                            Ok(()) => FetchOutcome::Hit(json),
                            Err(e) => FetchOutcome::Error(e.to_string()),
                        }
                    }
                    Err(e) => FetchOutcome::Error(format!("encode: {e}")),
                },
                Ok(None) => {
                    if let Err(e) =
                        set_negative_sentinel(&pool_arc, &key_owned, DEFAULT_NEGATIVE_TTL_MS).await
                    {
                        return FetchOutcome::Error(e.to_string());
                    }
                    FetchOutcome::Negative
                }
                Err(e) => FetchOutcome::Error(e.to_string()),
            }
        })
        .await
        .clone();

    // 3. Evict in-flight entry — every awaiter has its result by now.
    registry.inflight.remove(key);

    match outcome {
        FetchOutcome::Hit(json) => {
            let value: T = serde_json::from_value(json).map_err(|e| {
                sqlx::Error::Decode(Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
            })?;
            Ok(Some(value))
        }
        FetchOutcome::Negative => Ok(None),
        FetchOutcome::Error(msg) => Err(sqlx::Error::Protocol(msg)),
    }
}

fn decode_envelope_data<T: DeserializeOwned>(envelope: &serde_json::Value) -> Result<T, sqlx::Error> {
    let inner = envelope
        .get("data")
        .cloned()
        .ok_or_else(|| sqlx::Error::Protocol("cached envelope missing `data` field".to_string()))?;
    serde_json::from_value(inner).map_err(|e| {
        sqlx::Error::Decode(Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
    })
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    use super::*;
    use pellucid_db::open_in_memory;
    use serde::Deserialize;

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct SampleData {
        items: Vec<u32>,
    }

    #[tokio::test]
    async fn cold_cache_invokes_fetcher_once_under_concurrency() {
        let pool = open_in_memory().await.expect("pool");
        let registry = CoalesceRegistry::new();
        let calls = Arc::new(AtomicUsize::new(0));

        let mut handles = Vec::new();
        for _ in 0..32 {
            let pool = pool.clone();
            let registry = registry.clone();
            let calls = calls.clone();
            handles.push(tokio::spawn(async move {
                cached_fetch_json::<SampleData, _, _>(
                    &pool,
                    &registry,
                    "test:k",
                    CacheTier::Fast,
                    move || {
                        let calls = calls.clone();
                        async move {
                            calls.fetch_add(1, Ordering::SeqCst);
                            tokio::time::sleep(Duration::from_millis(20)).await;
                            Ok::<_, Box<dyn std::error::Error + Send + Sync>>(Some(SampleData {
                                items: vec![1, 2, 3],
                            }))
                        }
                    },
                )
                .await
            }));
        }

        for h in handles {
            let result = h.await.expect("join").expect("inner");
            assert_eq!(
                result,
                Some(SampleData { items: vec![1, 2, 3] })
            );
        }

        // SPEC-001 §7.2: exactly one fetcher invocation per coalesced cycle.
        assert_eq!(
            calls.load(Ordering::SeqCst),
            1,
            "expected exactly 1 fetcher call across 32 concurrent awaiters"
        );
        assert_eq!(registry.inflight_len(), 0, "registry must be drained after fetch");
    }

    #[tokio::test]
    async fn warm_cache_short_circuits_without_fetcher() {
        let pool = open_in_memory().await.expect("pool");
        let registry = CoalesceRegistry::new();
        let calls = Arc::new(AtomicUsize::new(0));

        let warm = |c: Arc<AtomicUsize>| {
            move || {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, Box<dyn std::error::Error + Send + Sync>>(Some(SampleData {
                        items: vec![10],
                    }))
                }
            }
        };

        let _ = cached_fetch_json::<SampleData, _, _>(
            &pool,
            &registry,
            "warm:k",
            CacheTier::Medium,
            warm(calls.clone()),
        )
        .await
        .expect("first call");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Subsequent reads must NOT call the fetcher.
        let _ = cached_fetch_json::<SampleData, _, _>(
            &pool,
            &registry,
            "warm:k",
            CacheTier::Medium,
            warm(calls.clone()),
        )
        .await
        .expect("second call");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "warm cache must skip fetcher");
    }

    #[tokio::test]
    async fn negative_result_writes_sentinel_and_short_circuits_subsequent_reads() {
        let pool = open_in_memory().await.expect("pool");
        let registry = CoalesceRegistry::new();
        let calls = Arc::new(AtomicUsize::new(0));

        let neg = |c: Arc<AtomicUsize>| {
            move || {
                let c = c.clone();
                async move {
                    c.fetch_add(1, Ordering::SeqCst);
                    Ok::<_, Box<dyn std::error::Error + Send + Sync>>(None)
                }
            }
        };

        let r = cached_fetch_json::<SampleData, _, _>(
            &pool,
            &registry,
            "neg:k",
            CacheTier::Fast,
            neg(calls.clone()),
        )
        .await
        .expect("first call");
        assert_eq!(r, None);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        // Second read inside the sentinel TTL must short-circuit.
        let r2 = cached_fetch_json::<SampleData, _, _>(
            &pool,
            &registry,
            "neg:k",
            CacheTier::Fast,
            neg(calls.clone()),
        )
        .await
        .expect("second call");
        assert_eq!(r2, None);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "negative sentinel must skip fetcher");
    }

    #[tokio::test]
    async fn fetcher_error_propagates_and_does_not_write_cache() {
        let pool = open_in_memory().await.expect("pool");
        let registry = CoalesceRegistry::new();

        let r: Result<Option<SampleData>, sqlx::Error> = cached_fetch_json(
            &pool,
            &registry,
            "err:k",
            CacheTier::Fast,
            move || async move {
                Err::<Option<SampleData>, _>(
                    Box::<dyn std::error::Error + Send + Sync>::from("upstream 503"),
                )
            },
        )
        .await;

        match r {
            Err(sqlx::Error::Protocol(msg)) => assert!(msg.contains("upstream 503")),
            other => panic!("expected Protocol(upstream), got {other:?}"),
        }

        // Cache must not have been written; a follow-up call goes through
        // again (no negative sentinel for errors).
        let row = sqlx::query("SELECT cache_key FROM kv_envelope WHERE cache_key = 'err:k'")
            .fetch_optional(&pool)
            .await
            .expect("query");
        assert!(row.is_none(), "errors must not write a cache row");
    }
}
