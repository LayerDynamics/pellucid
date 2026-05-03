// Fixture: cached_fetch_json where the cache-key argument is
// `&q.cache_key()` — a method call on a request receiver. The
// linter MUST trust the typed method as the canonical source
// of truth for the template and skip the field-comparison
// check. (The handler reads `q.flight` and `q.date`; the
// linter would over-flag if it didn't recognise the trust
// path.)

#![allow(dead_code, clippy::needless_pass_by_value)]

pub struct Pool;
pub struct Registry;
pub struct Tier;
pub struct Query {
    pub flight: String,
    pub date: String,
}

impl Query {
    pub fn cache_key(&self) -> String {
        format!("aviation:status:{}:{}:v1", self.flight, self.date)
    }
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
    let _ = q.flight;
    let _ = q.date;
    let key = q.cache_key();
    let _ = cached_fetch_json::<String, _, _>(
        pool,
        registry,
        &key,
        Tier,
        || async { Ok(None) },
    )
    .await;
    Ok(())
}
