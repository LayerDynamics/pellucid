// Fixture: cached_fetch_json with turbofish callee. The linter
// must recognise this as a cached_fetch_json call regardless
// of the type parameters wrapped in `::<...>`.
//
// This handler intentionally references `q.flight` but omits
// `flight` from the cache-key template — the linter MUST flag
// it. (The trust-path is intentionally not used here so the
// turbofish detection is exercised without being short-
// circuited.)

#![allow(dead_code, clippy::needless_pass_by_value)]

pub struct Pool;
pub struct Registry;
pub struct Tier;
pub struct Query {
    pub flight: String,
}

pub async fn cached_fetch_json<T, F, Fut>(
    _pool: &Pool,
    _registry: &Registry,
    _key: &str,
    _tier: Tier,
    _fetcher: F,
) -> Result<Option<T>, ()>
where
    T: Send,
    F: FnOnce() -> Fut + Send + 'static,
    Fut: std::future::Future<Output = Result<Option<T>, ()>> + Send,
{
    Ok(None)
}

pub async fn handler(pool: &Pool, registry: &Registry, q: Query) -> Result<(), ()> {
    let _ = q.flight; // referenced — must show up in cache key
    let _ = cached_fetch_json::<String, _, _>(
        pool,
        registry,
        "aviation:status:NO_FIELD:v1",
        Tier,
        || async { Ok(None) },
    )
    .await;
    Ok(())
}
