// Fixture: hardcoded cache key, no req-field accesses. Lints clean.
pub async fn handle(_req: ListMarketQuotesRequest) -> Result<Response, Error> {
    cached_fetch_json("market:stocks-bootstrap:v1", MEDIUM, fetch_upstream).await
}
