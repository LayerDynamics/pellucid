//! `GET /api/intelligence/v1/regional` handler.
//!
//! Aggregates the FAST-tier ACLED snapshot
//! (`conflict:events-24h:v1`) and the FAST-tier GDELT incident
//! feed (`conflict:incident-feed:v1`) into per-region rollups
//! the webview's `RegionalIntelligenceBoard` (T4.1.6) renders
//! as a board of region cards.
//!
//! ## Why a composing handler (no new seeder)
//!
//! Both upstream cache slots already carry country-keyed data.
//! Adding another seeder would mean another moving piece on
//! the relay; reading + folding two slots inside the handler
//! is ~free (in-process SQLite hits are sub-ms) and keeps the
//! seed pipeline unchanged.
//!
//! ## Region mapping
//!
//! Countries are folded into a small set of named regions
//! (`Middle East`, `Europe`, `East Asia`, …) via the
//! [`region_for_country`] helper. The mapping is deliberately
//! coarse — the panel surfaces it as orientation chrome, not
//! authoritative geography.
//!
//! ## Wire shape
//!
//! ```jsonc
//! {
//!   "regions": [{
//!     "region": "Middle East",
//!     "totalEvents": 124,
//!     "totalIncidents": 38,
//!     "countries": [
//!       { "name": "Israel", "events": 56, "incidents": 12 },
//!       …
//!     ],
//!     "topActors": ["Israeli Forces", "Hamas", …]
//!   }],
//!   "assembledAtMs": 1746360000000,
//!   "stale": false
//! }
//! ```

use std::collections::BTreeMap;

use axum::extract::{Query, State};
use axum::http::{HeaderValue, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use pellucid_cache::{get_cached_json, CacheHit};
use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;

use crate::state::AppState;

/// Cache key for the ACLED actor snapshot.
pub const ACLED_CACHE_KEY: &str = "conflict:events-24h:v1";
/// Cache key for the GDELT incident feed.
pub const GDELT_CACHE_KEY: &str = "conflict:incident-feed:v1";

/// SPEC-001 §11.2 default Retry-After.
pub const DEFAULT_RETRY_AFTER_SECS: u32 = 30;

/// Top-N actor names retained per region. Keeps the wire small;
/// the panel surfaces these as a chip row.
pub const TOP_ACTORS_PER_REGION: usize = 5;

/// Per-country rollup row inside one region.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CountryRollup {
    /// Country name as the upstreams reported it.
    pub name: String,
    /// ACLED event count attributed to this country across all
    /// actor rows.
    pub events: u64,
    /// GDELT article count whose `source_country` matches.
    pub incidents: u64,
}

/// One region's rollup.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct RegionRollup {
    /// Region label — one of [`KNOWN_REGIONS`] or `"Other"`.
    pub region: String,
    /// Sum of `events` across [`countries`].
    #[serde(rename = "totalEvents")]
    pub total_events: u64,
    /// Sum of `incidents` across [`countries`].
    #[serde(rename = "totalIncidents")]
    pub total_incidents: u64,
    /// Per-country breakdown, sorted descending by
    /// `events + incidents`.
    pub countries: Vec<CountryRollup>,
    /// Top-N actor names for this region (from ACLED actor
    /// rows whose country breakdown overlaps this region).
    #[serde(rename = "topActors")]
    pub top_actors: Vec<String>,
}

/// Wire-format response.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RegionalResponse {
    /// One row per region with at least one country present
    /// in either cache slot. Sorted descending by
    /// `total_events + total_incidents`.
    pub regions: Vec<RegionRollup>,
    /// Maximum of the two upstream `assembled_at_ms`
    /// timestamps so a stale read is observable.
    #[serde(rename = "assembledAtMs")]
    pub assembled_at_ms: i64,
    /// True when EITHER upstream returned a stale row. The
    /// panel surfaces a single "cached snapshot" footer
    /// regardless of which slot was stale.
    pub stale: bool,
}

/// Optional query knobs.
#[derive(Debug, Default, Deserialize)]
pub struct RegionalQuery {
    /// Filter to a single named region. Case-insensitive,
    /// trim-tolerant. Useful when the panel's "drill-in" view
    /// asks for one region only.
    #[serde(default)]
    pub region: Option<String>,
}

/// Internal — actor row shape stored by the ACLED seeder.
/// Field names match `pellucid_seeders::conflict::seed_acled::ActorRow`.
#[derive(Debug, Deserialize)]
struct AcledActorRow {
    actor: String,
    event_count: u64,
    #[allow(dead_code)]
    total_fatalities: i64,
    country_breakdown: Vec<(String, u64)>,
}

#[derive(Debug, Deserialize)]
struct AcledSnapshotPayload {
    rows: Vec<AcledActorRow>,
    #[serde(default, rename = "assembled_at_ms", alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

#[derive(Debug, Deserialize)]
struct GdeltArticleRow {
    source_country: String,
}

#[derive(Debug, Deserialize)]
struct GdeltSnapshotPayload {
    rows: Vec<GdeltArticleRow>,
    #[serde(default, rename = "assembled_at_ms", alias = "assembledAtMs")]
    assembled_at_ms: i64,
}

/// Errors the handler can produce.
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// Cache layer failure.
    #[error("cache failure: {0}")]
    Cache(String),
    /// Cache row exists but did not deserialise.
    #[error("cache shape: {0}")]
    Shape(String),
    /// Both upstream cache slots are empty.
    #[error("upstream is empty (M4 outage path)")]
    Outage {
        /// `Retry-After` header value.
        retry_after_secs: u32,
    },
}

impl HandlerError {
    /// Stable error code.
    #[must_use]
    pub const fn code(&self) -> &'static str {
        match self {
            Self::Cache(_) => "cache_failure",
            Self::Shape(_) => "cache_shape",
            Self::Outage { .. } => "bootstrap_upstream_empty",
        }
    }

    /// HTTP status.
    #[must_use]
    pub const fn status(&self) -> StatusCode {
        match self {
            Self::Cache(_) | Self::Shape(_) => StatusCode::BAD_GATEWAY,
            Self::Outage { .. } => StatusCode::SERVICE_UNAVAILABLE,
        }
    }
}

impl axum::response::IntoResponse for HandlerError {
    fn into_response(self) -> axum::response::Response {
        let mut body = serde_json::json!({
            "error": {
                "code": self.code(),
                "message": self.to_string(),
            }
        });
        if let Self::Outage { retry_after_secs } = &self {
            body["error"]["retry_after_secs"] = serde_json::Value::from(*retry_after_secs);
        }
        let status = self.status();
        let mut resp = (status, Json(body)).into_response();
        resp.headers_mut().insert(
            GATEWAY_ERROR_CODE_HEADER,
            HeaderValue::from_static(self.code()),
        );
        if let Self::Outage { retry_after_secs } = &self {
            if let Ok(v) = HeaderValue::from_str(&retry_after_secs.to_string()) {
                resp.headers_mut().insert("retry-after", v);
            }
        }
        resp
    }
}

/// Region labels the [`region_for_country`] mapping uses.
pub const KNOWN_REGIONS: &[&str] = &[
    "Middle East",
    "Europe",
    "East Asia",
    "South Asia",
    "Africa",
    "Americas",
    "Oceania",
];

/// Map a country name to a coarse region label. Unknown
/// countries fall into `"Other"`. The mapping covers the
/// 8-country ACLED basket
/// (`pellucid_seeders::conflict::seed_acled::DEFAULT_ISO_CODES`)
/// + the high-volume GDELT source countries.
#[must_use]
pub fn region_for_country(country: &str) -> &'static str {
    let needle = country.trim().to_ascii_lowercase();
    match needle.as_str() {
        // Middle East — ACLED basket: Saudi Arabia (760), Iran (368),
        // Yemen (887), Iraq (368→duplicate code in plan, use 364 = Israel
        // here), Israel (376), plus regional neighbours.
        "saudi arabia"
        | "iran"
        | "yemen"
        | "iraq"
        | "israel"
        | "syria"
        | "lebanon"
        | "jordan"
        | "egypt"
        | "turkey"
        | "uae"
        | "united arab emirates"
        | "qatar"
        | "bahrain"
        | "oman"
        | "palestine"
        | "kuwait" => "Middle East",
        // Europe — ACLED basket includes Ukraine (804); plus EU/UK.
        "ukraine" | "russia" | "russian federation" | "germany" | "france" | "united kingdom"
        | "uk" | "poland" | "spain" | "italy" | "netherlands" | "belgium" | "sweden" | "norway"
        | "finland" | "denmark" | "ireland" | "portugal" | "greece" | "switzerland" | "austria"
        | "czech republic" | "czechia" | "hungary" | "romania" | "bulgaria" | "serbia"
        | "croatia" | "slovakia" | "slovenia" | "estonia" | "latvia" | "lithuania" | "belarus"
        | "moldova" => "Europe",
        // East Asia.
        "china" | "japan" | "south korea" | "north korea" | "taiwan" | "mongolia" => "East Asia",
        // South Asia — Pakistan (586), Afghanistan (4 — outside basket
        // but high GDELT volume), India (356), Bangladesh, Sri Lanka.
        "pakistan" | "afghanistan" | "india" | "bangladesh" | "sri lanka" | "nepal" | "bhutan"
        | "maldives" => "South Asia",
        // Africa.
        "nigeria"
        | "south africa"
        | "kenya"
        | "ethiopia"
        | "sudan"
        | "south sudan"
        | "libya"
        | "tunisia"
        | "algeria"
        | "morocco"
        | "somalia"
        | "uganda"
        | "tanzania"
        | "ghana"
        | "cameroon"
        | "drc"
        | "democratic republic of the congo"
        | "central african republic"
        | "mali"
        | "burkina faso"
        | "niger"
        | "chad"
        | "rwanda"
        | "burundi"
        | "mozambique"
        | "zimbabwe"
        | "zambia"
        | "angola" => "Africa",
        // Americas.
        "united states" | "usa" | "us" | "canada" | "mexico" | "brazil" | "argentina"
        | "colombia" | "venezuela" | "chile" | "peru" | "ecuador" | "bolivia" | "paraguay"
        | "uruguay" | "guyana" | "suriname" | "haiti" | "cuba" | "dominican republic"
        | "guatemala" | "honduras" | "nicaragua" | "costa rica" | "panama" | "el salvador" => {
            "Americas"
        }
        // Oceania.
        "australia" | "new zealand" | "papua new guinea" | "fiji" => "Oceania",
        _ => "Other",
    }
}

/// Compose the per-region rollups from raw payloads. Pure —
/// extracted so unit tests pin every boundary.
#[must_use]
pub fn compose_rollups(
    acled: Option<AcledSnapshot>,
    gdelt: Option<GdeltSnapshot>,
) -> Vec<RegionRollup> {
    /// Per-country accumulator: ACLED event count, GDELT incident
    /// count, and an actor→event-count map used to compute the
    /// region's top-N actors after rollup.
    type CountryAcc = (u64, u64, BTreeMap<String, u64>);
    /// Per-region accumulator: country name → [`CountryAcc`].
    type RegionAcc = BTreeMap<String, CountryAcc>;

    let mut buckets: BTreeMap<&'static str, RegionAcc> = BTreeMap::new();

    if let Some(a) = acled {
        for row in a.rows {
            for (country, count) in row.country_breakdown {
                let region = region_for_country(&country);
                let entry = buckets
                    .entry(region)
                    .or_default()
                    .entry(country)
                    .or_insert_with(|| (0, 0, BTreeMap::new()));
                entry.0 += count;
                *entry.2.entry(row.actor.clone()).or_insert(0) += row.event_count;
            }
        }
    }
    if let Some(g) = gdelt {
        for row in g.rows {
            let region = region_for_country(&row.source_country);
            let entry = buckets
                .entry(region)
                .or_default()
                .entry(row.source_country)
                .or_insert_with(|| (0, 0, BTreeMap::new()));
            entry.1 += 1;
        }
    }

    let mut regions: Vec<RegionRollup> = buckets
        .into_iter()
        .map(|(region, country_map)| {
            let mut total_events: u64 = 0;
            let mut total_incidents: u64 = 0;
            let mut actor_totals: BTreeMap<String, u64> = BTreeMap::new();
            let mut countries: Vec<CountryRollup> = country_map
                .into_iter()
                .map(|(name, (events, incidents, actor_breakdown))| {
                    total_events = total_events.saturating_add(events);
                    total_incidents = total_incidents.saturating_add(incidents);
                    for (actor, c) in actor_breakdown {
                        *actor_totals.entry(actor).or_insert(0) += c;
                    }
                    CountryRollup {
                        name,
                        events,
                        incidents,
                    }
                })
                .collect();
            countries.sort_by(|a, b| {
                let lhs = b.events + b.incidents;
                let rhs = a.events + a.incidents;
                lhs.cmp(&rhs).then_with(|| a.name.cmp(&b.name))
            });
            let mut actor_pairs: Vec<(String, u64)> = actor_totals.into_iter().collect();
            actor_pairs.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
            let top_actors: Vec<String> = actor_pairs
                .into_iter()
                .take(TOP_ACTORS_PER_REGION)
                .map(|(name, _)| name)
                .collect();
            RegionRollup {
                region: region.to_string(),
                total_events,
                total_incidents,
                countries,
                top_actors,
            }
        })
        .collect();

    regions.sort_by(|a, b| {
        let lhs = b.total_events + b.total_incidents;
        let rhs = a.total_events + a.total_incidents;
        lhs.cmp(&rhs).then_with(|| a.region.cmp(&b.region))
    });
    regions
}

/// Apply the optional `?region=` filter. Pure.
#[must_use]
pub fn apply_region_filter(rollups: Vec<RegionRollup>, q: &RegionalQuery) -> Vec<RegionRollup> {
    if let Some(needle) = q.region.as_deref() {
        let key = needle.trim().to_ascii_lowercase();
        rollups
            .into_iter()
            .filter(|r| r.region.to_ascii_lowercase() == key)
            .collect()
    } else {
        rollups
    }
}

/// Public-facing snapshot wrappers — exposed so tests can build
/// payloads without re-typing the seeder field names.
#[derive(Clone, Debug)]
pub struct AcledSnapshot {
    /// Top-N actor rows with country breakdowns.
    pub rows: Vec<AcledActorRowOwned>,
    /// Wall-clock ms when the seeder assembled.
    pub assembled_at_ms: i64,
}

#[derive(Clone, Debug)]
pub struct AcledActorRowOwned {
    /// Actor name.
    pub actor: String,
    /// Total events across the basket window.
    pub event_count: u64,
    /// Country → event-count breakdown.
    pub country_breakdown: Vec<(String, u64)>,
}

#[derive(Clone, Debug)]
pub struct GdeltSnapshot {
    /// Article rows with their source-country tag.
    pub rows: Vec<GdeltArticleRowOwned>,
    /// Wall-clock ms when the seeder assembled.
    pub assembled_at_ms: i64,
}

#[derive(Clone, Debug)]
pub struct GdeltArticleRowOwned {
    /// GDELT-reported source country.
    pub source_country: String,
}

// Internal conversions so the public test types feed into the
// private deserialised types `compose_rollups` walks over.
impl From<AcledActorRowOwned> for AcledActorRow {
    fn from(v: AcledActorRowOwned) -> Self {
        Self {
            actor: v.actor,
            event_count: v.event_count,
            total_fatalities: 0,
            country_breakdown: v.country_breakdown,
        }
    }
}

impl From<AcledSnapshot> for AcledSnapshotPayload {
    fn from(v: AcledSnapshot) -> Self {
        Self {
            rows: v.rows.into_iter().map(Into::into).collect(),
            assembled_at_ms: v.assembled_at_ms,
        }
    }
}

impl From<GdeltArticleRowOwned> for GdeltArticleRow {
    fn from(v: GdeltArticleRowOwned) -> Self {
        Self {
            source_country: v.source_country,
        }
    }
}

impl From<GdeltSnapshot> for GdeltSnapshotPayload {
    fn from(v: GdeltSnapshot) -> Self {
        Self {
            rows: v.rows.into_iter().map(Into::into).collect(),
            assembled_at_ms: v.assembled_at_ms,
        }
    }
}

/// Axum handler.
pub async fn handler(
    State(state): State<AppState>,
    Query(q): Query<RegionalQuery>,
) -> Result<Json<RegionalResponse>, HandlerError> {
    let acled_raw = get_cached_json::<Value>(&state.pool, ACLED_CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;
    let gdelt_raw = get_cached_json::<Value>(&state.pool, GDELT_CACHE_KEY)
        .await
        .map_err(|e| HandlerError::Cache(e.to_string()))?;

    let (acled_payload, acled_stale) = decode_optional::<AcledSnapshotPayload>(acled_raw)?;
    let (gdelt_payload, gdelt_stale) = decode_optional::<GdeltSnapshotPayload>(gdelt_raw)?;

    if acled_payload.is_none() && gdelt_payload.is_none() {
        return Err(HandlerError::Outage {
            retry_after_secs: DEFAULT_RETRY_AFTER_SECS,
        });
    }

    let acled_assembled = acled_payload.as_ref().map_or(0, |p| p.assembled_at_ms);
    let gdelt_assembled = gdelt_payload.as_ref().map_or(0, |p| p.assembled_at_ms);
    let assembled_at_ms = acled_assembled.max(gdelt_assembled);

    let acled_snap = acled_payload.map(|p| AcledSnapshot {
        assembled_at_ms: p.assembled_at_ms,
        rows: p
            .rows
            .into_iter()
            .map(|r| AcledActorRowOwned {
                actor: r.actor,
                event_count: r.event_count,
                country_breakdown: r.country_breakdown,
            })
            .collect(),
    });
    let gdelt_snap = gdelt_payload.map(|p| GdeltSnapshot {
        assembled_at_ms: p.assembled_at_ms,
        rows: p
            .rows
            .into_iter()
            .map(|r| GdeltArticleRowOwned {
                source_country: r.source_country,
            })
            .collect(),
    });

    let rollups = compose_rollups(acled_snap, gdelt_snap);
    let regions = apply_region_filter(rollups, &q);
    Ok(Json(RegionalResponse {
        regions,
        assembled_at_ms,
        stale: acled_stale || gdelt_stale,
    }))
}

/// Decode an optional cache slot. Returns `(payload, stale)`.
/// `payload` is `None` only when the slot is missing or carries
/// the negative sentinel.
fn decode_optional<T>(raw: CacheHit<Value>) -> Result<(Option<T>, bool), HandlerError>
where
    T: for<'de> serde::Deserialize<'de>,
{
    let (value, stale) = match raw {
        CacheHit::Fresh(v) => (v, false),
        CacheHit::Stale(v) => (v, true),
        CacheHit::NegativeSentinel | CacheHit::Miss => return Ok((None, false)),
    };
    let inner = unwrap_envelope_data(value);
    let parsed: T =
        serde_json::from_value(inner).map_err(|e| HandlerError::Shape(e.to_string()))?;
    Ok((Some(parsed), stale))
}

/// Same envelope-unwrap helper used by the news + intel handlers.
fn unwrap_envelope_data(v: Value) -> Value {
    if let Value::Object(map) = &v {
        if map.contains_key("_seed") {
            if let Some(inner) = map.get("data") {
                return inner.clone();
            }
        }
    }
    v
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::intelligence::v1::REGIONAL_PATH;
    use axum::body::Body;
    use axum::http::{Request, StatusCode as Code};
    use pellucid_cache::set_cached_json;
    use pellucid_core::Envelope;
    use tower::ServiceExt;

    async fn migrated_router() -> (axum::Router, pellucid_db::Pool) {
        let state = AppState::for_tests_async().await.unwrap();
        let pool = state.pool.clone();
        let app =
            axum::Router::new().route(REGIONAL_PATH, axum::routing::get(handler).with_state(state));
        (app, pool)
    }

    /// One ACLED actor row in the test seeder fixture: (actor,
    /// event_count, country_breakdown).
    type AcledTestRow<'a> = (&'a str, u64, Vec<(&'a str, u64)>);

    fn acled_value(rows: Vec<AcledTestRow<'_>>) -> Value {
        serde_json::json!({
            "rows": rows.into_iter().map(|(actor, ec, cb)| serde_json::json!({
                "actor": actor,
                "event_count": ec,
                "total_fatalities": 0,
                "country_breakdown": cb.into_iter().map(|(c, n)| (c.to_string(), n)).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
            "total_events": 0,
            "date_range": ["2026-04-28", "2026-05-05"],
            "assembled_at_ms": 1_700_000_000_000_i64,
        })
    }

    fn gdelt_value(countries: &[&str]) -> Value {
        serde_json::json!({
            "rows": countries.iter().map(|c| serde_json::json!({
                "url": "https://x",
                "title": "x",
                "seen_date": "20260504T120000Z",
                "social_image": "",
                "domain": "x",
                "language": "English",
                "source_country": c,
            })).collect::<Vec<_>>(),
            "query": "(theme:KILL)",
            "timespan": "24h",
            "assembled_at_ms": 1_700_000_001_000_i64,
        })
    }

    #[test]
    fn region_mapping_handles_known_basket_countries() {
        assert_eq!(region_for_country("Iran"), "Middle East");
        assert_eq!(region_for_country("  iran  "), "Middle East");
        assert_eq!(region_for_country("Ukraine"), "Europe");
        assert_eq!(region_for_country("China"), "East Asia");
        assert_eq!(region_for_country("India"), "South Asia");
        assert_eq!(region_for_country("Nigeria"), "Africa");
        assert_eq!(region_for_country("United States"), "Americas");
        assert_eq!(region_for_country("Australia"), "Oceania");
    }

    #[test]
    fn region_mapping_unknown_country_falls_back_to_other() {
        assert_eq!(region_for_country("Atlantis"), "Other");
        assert_eq!(region_for_country(""), "Other");
    }

    #[test]
    fn compose_rollups_aggregates_acled_country_breakdowns() {
        let acled = AcledSnapshot {
            assembled_at_ms: 1,
            rows: vec![AcledActorRowOwned {
                actor: "Hamas".into(),
                event_count: 10,
                country_breakdown: vec![("Israel".into(), 5), ("Iran".into(), 3)],
            }],
        };
        let rollups = compose_rollups(Some(acled), None);
        let me = rollups
            .iter()
            .find(|r| r.region == "Middle East")
            .expect("middle east rollup");
        assert_eq!(me.total_events, 8);
        assert_eq!(me.total_incidents, 0);
        assert!(me.top_actors.contains(&"Hamas".to_string()));
        let names: Vec<&str> = me.countries.iter().map(|c| c.name.as_str()).collect();
        assert!(names.contains(&"Israel"));
        assert!(names.contains(&"Iran"));
    }

    #[test]
    fn compose_rollups_counts_gdelt_incidents_per_country() {
        let gdelt = GdeltSnapshot {
            assembled_at_ms: 1,
            rows: vec![
                GdeltArticleRowOwned {
                    source_country: "Iran".into(),
                },
                GdeltArticleRowOwned {
                    source_country: "Iran".into(),
                },
                GdeltArticleRowOwned {
                    source_country: "Israel".into(),
                },
                GdeltArticleRowOwned {
                    source_country: "China".into(),
                },
            ],
        };
        let rollups = compose_rollups(None, Some(gdelt));
        let me = rollups
            .iter()
            .find(|r| r.region == "Middle East")
            .expect("middle east");
        assert_eq!(me.total_incidents, 3);
        let ea = rollups
            .iter()
            .find(|r| r.region == "East Asia")
            .expect("east asia");
        assert_eq!(ea.total_incidents, 1);
    }

    #[test]
    fn compose_rollups_sorts_regions_by_total_volume_desc() {
        let acled = AcledSnapshot {
            assembled_at_ms: 1,
            rows: vec![
                AcledActorRowOwned {
                    actor: "x".into(),
                    event_count: 1,
                    country_breakdown: vec![("Iran".into(), 100)],
                },
                AcledActorRowOwned {
                    actor: "y".into(),
                    event_count: 1,
                    country_breakdown: vec![("China".into(), 5)],
                },
            ],
        };
        let rollups = compose_rollups(Some(acled), None);
        // Middle East (100) should come before East Asia (5).
        assert_eq!(rollups[0].region, "Middle East");
        assert_eq!(rollups[1].region, "East Asia");
    }

    #[test]
    fn compose_rollups_caps_top_actors_per_region() {
        let acled = AcledSnapshot {
            assembled_at_ms: 1,
            rows: (0..(TOP_ACTORS_PER_REGION + 5))
                .map(|i| AcledActorRowOwned {
                    actor: format!("actor{i}"),
                    event_count: (i as u64) + 1,
                    country_breakdown: vec![("Iran".into(), 1)],
                })
                .collect(),
        };
        let rollups = compose_rollups(Some(acled), None);
        let me = rollups.iter().find(|r| r.region == "Middle East").unwrap();
        assert_eq!(me.top_actors.len(), TOP_ACTORS_PER_REGION);
    }

    #[test]
    fn apply_region_filter_is_case_insensitive() {
        let rollups = vec![
            RegionRollup {
                region: "Middle East".into(),
                total_events: 1,
                total_incidents: 0,
                countries: vec![],
                top_actors: vec![],
            },
            RegionRollup {
                region: "Europe".into(),
                total_events: 2,
                total_incidents: 0,
                countries: vec![],
                top_actors: vec![],
            },
        ];
        let q = RegionalQuery {
            region: Some(" middle east ".into()),
        };
        let out = apply_region_filter(rollups, &q);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].region, "Middle East");
    }

    #[tokio::test]
    async fn handler_returns_503_when_both_caches_empty() {
        let (app, _pool) = migrated_router().await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(REGIONAL_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::SERVICE_UNAVAILABLE);
        assert_eq!(resp.headers().get("retry-after").unwrap(), "30");
    }

    #[tokio::test]
    async fn handler_serves_when_only_one_cache_slot_present() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(acled_value(vec![("Hamas", 5, vec![("Israel", 5)])]));
        set_cached_json(&pool, ACLED_CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(REGIONAL_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: RegionalResponse = serde_json::from_slice(&body).unwrap();
        assert!(!parsed.regions.is_empty());
    }

    #[tokio::test]
    async fn handler_combines_both_caches_into_one_rollup_per_region() {
        let (app, pool) = migrated_router().await;
        let acled_env = Envelope::new(acled_value(vec![(
            "Hamas",
            10,
            vec![("Israel", 5), ("Iran", 3)],
        )]));
        let gdelt_env = Envelope::new(gdelt_value(&["Iran", "Iran", "Israel"]));
        set_cached_json(&pool, ACLED_CACHE_KEY, &acled_env, 60_000)
            .await
            .unwrap();
        set_cached_json(&pool, GDELT_CACHE_KEY, &gdelt_env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(REGIONAL_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: RegionalResponse = serde_json::from_slice(&body).unwrap();
        let me = parsed
            .regions
            .iter()
            .find(|r| r.region == "Middle East")
            .unwrap();
        assert_eq!(me.total_events, 8);
        assert_eq!(me.total_incidents, 3);
        // assembled_at_ms is the max of the two upstream stamps.
        assert_eq!(parsed.assembled_at_ms, 1_700_000_001_000);
    }

    #[tokio::test]
    async fn handler_filter_by_region_query_param_returns_one_row() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(acled_value(vec![
            ("a1", 1, vec![("Iran", 5)]),
            ("a2", 1, vec![("China", 5)]),
        ]));
        set_cached_json(&pool, ACLED_CACHE_KEY, &env, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(format!("{REGIONAL_PATH}?region=Middle%20East"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: RegionalResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(parsed.regions.len(), 1);
        assert_eq!(parsed.regions[0].region, "Middle East");
    }

    #[tokio::test]
    async fn handler_marks_stale_when_either_cache_slot_is_stale() {
        let (app, pool) = migrated_router().await;
        let env = Envelope::new(acled_value(vec![("a", 1, vec![("Iran", 1)])]));
        set_cached_json(&pool, ACLED_CACHE_KEY, &env, 0)
            .await
            .unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(REGIONAL_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::OK);
        let body = axum::body::to_bytes(resp.into_body(), 1_000_000)
            .await
            .unwrap();
        let parsed: RegionalResponse = serde_json::from_slice(&body).unwrap();
        assert!(parsed.stale);
    }

    #[tokio::test]
    async fn handler_returns_502_on_shape_mismatch() {
        let (app, pool) = migrated_router().await;
        let bad = Envelope::new(serde_json::json!({ "rows": "not-an-array" }));
        set_cached_json(&pool, ACLED_CACHE_KEY, &bad, 60_000)
            .await
            .unwrap();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(REGIONAL_PATH)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), Code::BAD_GATEWAY);
    }
}
