// Fixture: handler reads req.flight but the cache key omits it.
// MUST be flagged by check-cache-keys.

pub async fn handle(req: GetFlightStatusRequest) -> Result<Response, Error> {
    let _flight = &req.flight;
    let key = "aviation:status:fixed:v1";
    cached_fetch_json(key, FAST, fetch_upstream).await
}
