// Fixture: param-scoped cache key includes every `req.<field>` access.
// Lints clean.

pub async fn handle(req: GetFlightStatusRequest) -> Result<Response, Error> {
    let key = format!("aviation:status:{flight}:{date}:{origin}:v1",
        flight = req.flight,
        date = req.date,
        origin = req.origin,
    );
    cached_fetch_json(&key, FAST, fetch_upstream).await
}
