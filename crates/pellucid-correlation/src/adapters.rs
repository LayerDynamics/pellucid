//! Four [`Correlator`] impls — `Military`, `Escalation`, `Economic`,
//! `Disaster` — porting the constants + `generateTitle` logic from
//! `worldmonitor/src/services/correlation-engine/adapters/*.ts`.
//!
//! Each adapter is stateless apart from the cached `weights`
//! HashMap — `Correlator::weights` returns a `&HashMap<String, f64>`
//! so the engine can use it without lifetimes leaking. Construct
//! once at startup; share `&` across `CorrelationEngine::run` calls.
//!
//! What's intentionally NOT here: `collectSignals`. The JS
//! `DomainAdapter::collectSignals` reads from the webview-side
//! `AppContext.intelligenceCache` (military.flights, vessels,
//! protests.events, outages, latest news clusters, latest market
//! quotes, earthquakes, …) — those cache shapes belong on the
//! handler / IPC side, not in the algorithm crate. The handler
//! glue in `correlation/v1/run` will assemble `Vec<SignalEvidence>`
//! from cached envelopes (`military:active-deployments:v1`,
//! `events:protests:v1`, etc.) and pass them to
//! `CorrelationEngine::run(adapter, signals, now)`.

use std::collections::HashMap;
use std::collections::HashSet;

use once_cell::sync::Lazy;
use regex::Regex;

use crate::engine::{ClusterMode, CorrelationDomain, Correlator, SignalEvidence};

// ============================================================================
// Military adapter — port of `military.ts`
// ============================================================================

/// Strike-aircraft type tags. Matches `military.ts:13`. Exposed so
/// the handler-side `collect_signals` can use the same set when
/// stamping severities on emitted `military_flight` evidence.
pub const STRIKE_TYPES: &[&str] = &["fighter", "bomber", "attack"];

/// Support-aircraft type tags. Matches `military.ts:14`.
pub const SUPPORT_TYPES: &[&str] = &["tanker", "awacs", "surveillance", "electronic_warfare"];

/// Military adapter — geographic clustering with a 500 km radius
/// over 24 h. Threshold 20.
#[derive(Debug)]
pub struct MilitaryAdapter {
    weights: HashMap<String, f64>,
}

impl Default for MilitaryAdapter {
    fn default() -> Self {
        let mut w = HashMap::new();
        // Renormalised v1 weights from `military.ts:7-11`. Sum = 1.0.
        w.insert("military_flight".into(), 0.40);
        w.insert("ais_gap".into(), 0.30);
        w.insert("military_vessel".into(), 0.30);
        Self { weights: w }
    }
}

impl MilitaryAdapter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Correlator for MilitaryAdapter {
    fn domain(&self) -> CorrelationDomain {
        CorrelationDomain::Military
    }
    fn label(&self) -> &str {
        "Force Posture"
    }
    fn cluster_mode(&self) -> ClusterMode {
        ClusterMode::Geographic
    }
    fn spatial_radius_km(&self) -> f64 {
        500.0
    }
    fn time_window_hours(&self) -> u32 {
        24
    }
    fn threshold(&self) -> u32 {
        20
    }
    fn weights(&self) -> &HashMap<String, f64> {
        &self.weights
    }

    /// Title rules from `military.ts:99-121`:
    /// - `Strike packaging detected — <countries>` when both a
    ///   strike type AND a support type are present in the
    ///   cluster's `military_flight` aircraft types.
    /// - `Combined air-naval activity — <countries>` when both
    ///   `military_flight` and `military_vessel` types are present.
    /// - Single-type fallbacks for flights-only or vessels-only.
    /// - `Military activity convergence — <countries>` otherwise.
    fn generate_title(
        &self,
        signals: &[SignalEvidence],
        _country: Option<&str>,
        _entity_key: Option<&str>,
    ) -> String {
        let types: HashSet<&str> = signals.iter().map(|s| s.signal_type.as_str()).collect();
        let countries = first_two_unique_countries(signals);
        let country_label = if countries.is_empty() {
            "Unknown region".to_string()
        } else {
            countries.join("/")
        };

        let has_flights = types.contains("military_flight");
        let has_vessels = types.contains("military_vessel");

        // Aircraft-type set drawn from raw_data.aircraftType on the
        // `military_flight` evidence. The handler-side collector
        // populates this; if it's missing we just don't trigger the
        // strike-package title.
        let mut flight_types: HashSet<String> = HashSet::new();
        for s in signals
            .iter()
            .filter(|s| s.signal_type == "military_flight")
        {
            if let Some(at) = s
                .raw_data
                .as_ref()
                .and_then(|v| v.get("aircraftType"))
                .and_then(|v| v.as_str())
            {
                flight_types.insert(at.to_string());
            }
        }
        let has_strike = STRIKE_TYPES.iter().any(|t| flight_types.contains(*t));
        let has_support = SUPPORT_TYPES.iter().any(|t| flight_types.contains(*t));
        let has_strike_package = has_strike && has_support;

        if has_strike_package {
            return format!("Strike packaging detected \u{2014} {country_label}");
        }
        if has_flights && has_vessels {
            return format!("Combined air-naval activity \u{2014} {country_label}");
        }
        if has_flights {
            return format!("Military flight cluster \u{2014} {country_label}");
        }
        if has_vessels {
            return format!("Naval vessel concentration \u{2014} {country_label}");
        }
        format!("Military activity convergence \u{2014} {country_label}")
    }
}

// ============================================================================
// Escalation adapter — port of `escalation.ts`
// ============================================================================

/// Country-mode adapter for conflict-escalation signals over a
/// 48-hour window. Threshold 20.
#[derive(Debug)]
pub struct EscalationAdapter {
    weights: HashMap<String, f64>,
}

impl Default for EscalationAdapter {
    fn default() -> Self {
        let mut w = HashMap::new();
        // Renormalised v1 weights from `escalation.ts:7-11`. Sum = 1.0.
        w.insert("conflict_event".into(), 0.45);
        w.insert("escalation_outage".into(), 0.25);
        w.insert("news_severity".into(), 0.30);
        Self { weights: w }
    }
}

impl EscalationAdapter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Correlator for EscalationAdapter {
    fn domain(&self) -> CorrelationDomain {
        CorrelationDomain::Escalation
    }
    fn label(&self) -> &str {
        "Escalation Monitor"
    }
    fn cluster_mode(&self) -> ClusterMode {
        ClusterMode::Country
    }
    fn spatial_radius_km(&self) -> f64 {
        0.0
    }
    fn time_window_hours(&self) -> u32 {
        48
    }
    fn threshold(&self) -> u32 {
        20
    }
    fn weights(&self) -> &HashMap<String, f64> {
        &self.weights
    }

    /// Title rules from `escalation.ts:136-150`. The country label
    /// is the supplied ISO-2 code (the handler can re-map to a
    /// display name client-side). The JS uses
    /// `getCountryNameByCode` here; we leave that to the caller
    /// because we don't carry the country-name registry in this
    /// crate.
    fn generate_title(
        &self,
        signals: &[SignalEvidence],
        country: Option<&str>,
        _entity_key: Option<&str>,
    ) -> String {
        let types: HashSet<&str> = signals.iter().map(|s| s.signal_type.as_str()).collect();
        let country_label = country.unwrap_or("Unknown");
        let mut parts: Vec<&str> = Vec::new();
        if types.contains("conflict_event") {
            parts.push("conflict");
        }
        if types.contains("escalation_outage") {
            parts.push("comms disruption");
        }
        if types.contains("news_severity") {
            parts.push("news escalation");
        }
        if parts.is_empty() {
            format!("Escalation signals \u{2014} {country_label}")
        } else {
            format!("{} \u{2014} {country_label}", parts.join(" + "))
        }
    }
}

// ============================================================================
// Economic adapter — port of `economic.ts`
// ============================================================================

/// Commodity / FX symbols recognised by the economic adapter.
/// Matches `economic.ts:12`.
pub static COMMODITY_SYMBOLS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        "CL=F", "GC=F", "NG=F", "SI=F", "HG=F", "ZW=F", "BTC-USD", "BZ=F", "ETH-USD", "KC=F",
        "SB=F", "CT=F", "CC=F",
    ]
    .into_iter()
    .collect()
});

/// Min absolute % change for a `market_move` / `commodity_spike`.
/// Matches `economic.ts:13`.
pub const SIGNIFICANT_CHANGE_PCT: f64 = 1.5;

/// Sanctions / trade-war keywords. Matches `economic.ts:11`.
pub static SANCTIONS_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(sanction|tariff|embargo|trade\s+war|ban|restrict|block|seize|freeze\s+assets|export\s+control|blacklist|decouple|decoupl|subsid|dumping|countervail|quota|levy|excise|retaliat|currency\s+manipulat|capital\s+controls|swift|cbdc|petrodollar|de-?dollar|opec|cartel|price\s+cap|oil|crude|commodity|shortage|stockpile|strategic\s+reserve|supply\s+chain|rare\s+earth|chip\s+ban|semiconductor|economic\s+warfare|financial\s+weapon)\b",
    )
    .unwrap_or_else(|e| {
        tracing::error!(target: "pellucid::correlation", "SANCTIONS_REGEX invalid: {e}");
        never_match()
    })
});

/// Country / actor regex used to extract the headline-level
/// "mentioned entity" for economic title rendering. Matches
/// `economic.ts:131`.
pub static KNOWN_ENTITIES_REGEX: Lazy<Regex> = Lazy::new(|| {
    Regex::new(
        r"(?i)\b(Iran|Russia|China|North Korea|Venezuela|Cuba|Syria|Myanmar|Belarus|Turkey|Saudi|OPEC|EU|USA?|United States|India)\b",
    )
    .unwrap_or_else(|e| {
        tracing::error!(target: "pellucid::correlation", "KNOWN_ENTITIES_REGEX invalid: {e}");
        never_match()
    })
});

/// Static-literal fallback for the two `Lazy<Regex>` blocks. The
/// `unwrap_or_else` arms call this rather than `.expect(...)` so
/// `clippy::expect_used` stays clean. `[a&&b]` is the intersection
/// of two disjoint character classes — guaranteed never-match.
fn never_match() -> Regex {
    Regex::new("[a&&b]").unwrap_or_else(|_| match Regex::new("a") {
        Ok(r) => r,
        Err(_) => unreachable!("`a` is the simplest possible regex literal"),
    })
}

/// Generic single-key entity tags whose `displayEntity` mapping
/// suppresses to empty (see `economic.ts:141-150`).
static GENERIC_ENTITY_KEYS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        "sanctions",
        "trade",
        "tariff",
        "commodity",
        "currency",
        "energy",
        "embargo",
        "semiconductor",
        "crypto",
        "inflation",
    ]
    .into_iter()
    .collect()
});

fn display_entity(key: Option<&str>) -> String {
    let Some(k) = key else { return String::new() };
    if GENERIC_ENTITY_KEYS.contains(k) {
        return String::new();
    }
    let mut chars = k.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

/// Extract the first headline-level entity match from a list of
/// labels. Mirrors `economic.ts:133-138`.
fn extract_mentioned_entity(labels: &[&str]) -> String {
    for label in labels {
        if let Some(m) = KNOWN_ENTITIES_REGEX.find(label) {
            return m.as_str().to_string();
        }
    }
    String::new()
}

#[derive(Debug)]
pub struct EconomicAdapter {
    weights: HashMap<String, f64>,
}

impl Default for EconomicAdapter {
    fn default() -> Self {
        let mut w = HashMap::new();
        // Weights from `economic.ts:5-9`. Sum = 1.0.
        w.insert("market_move".into(), 0.35);
        w.insert("sanctions_news".into(), 0.30);
        w.insert("commodity_spike".into(), 0.35);
        Self { weights: w }
    }
}

impl EconomicAdapter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Correlator for EconomicAdapter {
    fn domain(&self) -> CorrelationDomain {
        CorrelationDomain::Economic
    }
    fn label(&self) -> &str {
        "Economic Warfare"
    }
    fn cluster_mode(&self) -> ClusterMode {
        ClusterMode::Entity
    }
    fn spatial_radius_km(&self) -> f64 {
        0.0
    }
    fn time_window_hours(&self) -> u32 {
        24
    }
    fn threshold(&self) -> u32 {
        20
    }
    fn weights(&self) -> &HashMap<String, f64> {
        &self.weights
    }

    /// Title rules from `economic.ts:76-128`:
    /// - When the cluster has `sanctions_news` AND another type:
    ///   `Sanctions tightening: <country/entity>`.
    /// - When pure markets (no sanctions): `Market disruption:
    ///   <symbol>/<symbol>`.
    /// - Fallback: `Economic convergence: <entity>` when entity
    ///   is non-generic, else `Economic convergence detected`.
    fn generate_title(
        &self,
        signals: &[SignalEvidence],
        _country: Option<&str>,
        entity_key: Option<&str>,
    ) -> String {
        let types: HashSet<&str> = signals.iter().map(|s| s.signal_type.as_str()).collect();
        let has_sanctions = types.contains("sanctions_news");
        let has_market = types.contains("market_move") || types.contains("commodity_spike");

        if has_sanctions && has_market {
            // Try to surface a country / actor mentioned in any
            // signal label.
            let labels: Vec<&str> = signals.iter().map(|s| s.label.as_str()).collect();
            let entity = extract_mentioned_entity(&labels);
            let suffix = if entity.is_empty() {
                display_entity(entity_key)
            } else {
                entity
            };
            if suffix.is_empty() {
                return "Sanctions tightening".into();
            }
            return format!("Sanctions tightening: {suffix}");
        }
        if has_market && !has_sanctions {
            // Up to two distinct symbols / display names. The
            // handler emits these in `raw_data.display` /
            // `raw_data.symbol`; fall back to the first whitespace
            // token of the label if neither is present.
            let mut names: Vec<String> = Vec::new();
            for s in signals
                .iter()
                .filter(|s| s.signal_type == "market_move" || s.signal_type == "commodity_spike")
            {
                let name = s
                    .raw_data
                    .as_ref()
                    .and_then(|v| v.get("display"))
                    .and_then(|v| v.as_str())
                    .or_else(|| {
                        s.raw_data
                            .as_ref()
                            .and_then(|v| v.get("symbol"))
                            .and_then(|v| v.as_str())
                    })
                    .map(str::to_string)
                    .unwrap_or_else(|| {
                        s.label
                            .split_whitespace()
                            .next()
                            .unwrap_or(&s.label)
                            .to_string()
                    });
                if !names.iter().any(|n| n == &name) {
                    names.push(name);
                }
                if names.len() >= 2 {
                    break;
                }
            }
            return format!("Market disruption: {}", names.join("/"));
        }

        let fallback = display_entity(entity_key);
        if fallback.is_empty() {
            "Economic convergence detected".into()
        } else {
            format!("Economic convergence: {fallback}")
        }
    }
}

// ============================================================================
// Disaster adapter — port of `disaster.ts`
// ============================================================================

#[derive(Debug)]
pub struct DisasterAdapter {
    weights: HashMap<String, f64>,
}

impl Default for DisasterAdapter {
    fn default() -> Self {
        let mut w = HashMap::new();
        // Weights from `disaster.ts:6-9`. Sum = 1.0.
        w.insert("earthquake".into(), 0.55);
        w.insert("infra_outage".into(), 0.45);
        Self { weights: w }
    }
}

impl DisasterAdapter {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }
}

impl Correlator for DisasterAdapter {
    fn domain(&self) -> CorrelationDomain {
        CorrelationDomain::Disaster
    }
    fn label(&self) -> &str {
        "Disaster Cascade"
    }
    fn cluster_mode(&self) -> ClusterMode {
        ClusterMode::Geographic
    }
    fn spatial_radius_km(&self) -> f64 {
        500.0
    }
    fn time_window_hours(&self) -> u32 {
        96
    }
    fn threshold(&self) -> u32 {
        20
    }
    fn weights(&self) -> &HashMap<String, f64> {
        &self.weights
    }

    /// Title rules from `disaster.ts:89-108`. Reads earthquake
    /// magnitude from `raw_data.magnitude` and the earthquake's
    /// place name from after the em-dash in `label` (matches the
    /// JS pattern).
    fn generate_title(
        &self,
        signals: &[SignalEvidence],
        _country: Option<&str>,
        _entity_key: Option<&str>,
    ) -> String {
        let types: HashSet<&str> = signals.iter().map(|s| s.signal_type.as_str()).collect();
        let mut parts: Vec<String> = Vec::new();

        if types.contains("earthquake") {
            let max_mag = signals
                .iter()
                .filter(|s| s.signal_type == "earthquake")
                .filter_map(|s| {
                    s.raw_data
                        .as_ref()
                        .and_then(|v| v.get("magnitude"))
                        .and_then(|v| v.as_f64())
                })
                .fold(0.0_f64, f64::max);
            parts.push(format!("M{max_mag:.1} seismic"));
        }
        if types.contains("infra_outage") {
            parts.push("infra disruption".into());
        }

        let quake_place = signals
            .iter()
            .find(|s| s.signal_type == "earthquake")
            .and_then(|s| s.label.split('\u{2014}').nth(1))
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        if parts.is_empty() {
            "Disaster convergence detected".into()
        } else {
            let joined = parts.join(" + ");
            match quake_place {
                Some(place) => format!("Disaster cascade: {joined} \u{2014} {place}"),
                None => format!("Disaster cascade: {joined}"),
            }
        }
    }
}

// ============================================================================
// Helpers
// ============================================================================

fn first_two_unique_countries(signals: &[SignalEvidence]) -> Vec<String> {
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for s in signals {
        if let Some(c) = s.country.clone() {
            if seen.insert(c.clone()) {
                out.push(c);
                if out.len() == 2 {
                    break;
                }
            }
        }
    }
    out
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sig(ty: &str, country: Option<&str>, label: &str) -> SignalEvidence {
        SignalEvidence {
            signal_type: ty.into(),
            source: "t".into(),
            severity: 60,
            lat: None,
            lon: None,
            country: country.map(str::to_string),
            timestamp: 0,
            label: label.into(),
            raw_data: None,
        }
    }

    fn sig_raw(ty: &str, label: &str, raw: serde_json::Value) -> SignalEvidence {
        SignalEvidence {
            signal_type: ty.into(),
            source: "t".into(),
            severity: 60,
            lat: None,
            lon: None,
            country: None,
            timestamp: 0,
            label: label.into(),
            raw_data: Some(raw),
        }
    }

    // ── military ───────────────────────────────────────────

    #[test]
    fn military_constants_match_source() {
        let a = MilitaryAdapter::new();
        assert_eq!(a.domain(), CorrelationDomain::Military);
        assert_eq!(a.label(), "Force Posture");
        assert_eq!(a.cluster_mode(), ClusterMode::Geographic);
        assert!((a.spatial_radius_km() - 500.0).abs() < f64::EPSILON);
        assert_eq!(a.time_window_hours(), 24);
        assert_eq!(a.threshold(), 20);
        let w = a.weights();
        assert_eq!(w.len(), 3);
        // Weights renormalised to sum to 1.0.
        let sum: f64 = w.values().sum();
        assert!((sum - 1.0).abs() < 1e-9, "weights sum {sum}, expected 1.0");
    }

    #[test]
    fn military_strike_packaging_when_strike_and_support_present() {
        let signals = vec![
            sig_raw(
                "military_flight",
                "USAF F-22 raptor",
                json!({"aircraftType": "fighter"}),
            ),
            sig_raw(
                "military_flight",
                "USAF KC-135 stratotanker",
                json!({"aircraftType": "tanker"}),
            ),
        ];
        let title = MilitaryAdapter::new().generate_title(&signals, None, None);
        assert!(title.starts_with("Strike packaging detected"));
    }

    #[test]
    fn military_combined_air_naval_when_both_types_present() {
        let signals = vec![
            sig_raw(
                "military_flight",
                "F-15",
                json!({"aircraftType": "fighter"}),
            ),
            sig("military_vessel", Some("US"), "Carl Vinson"),
        ];
        let title = MilitaryAdapter::new().generate_title(&signals, None, None);
        // Strike-only, no support → not strike-package; flight +
        // vessel → combined.
        assert!(
            title.contains("Combined air-naval activity"),
            "got: {title}"
        );
    }

    #[test]
    fn military_flight_only_title() {
        let signals = vec![sig_raw(
            "military_flight",
            "F-15",
            json!({"aircraftType": "fighter"}),
        )];
        let title = MilitaryAdapter::new().generate_title(&signals, None, None);
        assert!(title.contains("Military flight cluster"));
    }

    #[test]
    fn military_vessel_only_title() {
        let signals = vec![sig("military_vessel", Some("US"), "Carl Vinson")];
        let title = MilitaryAdapter::new().generate_title(&signals, None, None);
        assert!(title.contains("Naval vessel concentration"));
    }

    #[test]
    fn military_country_label_uses_first_two_unique() {
        let signals = vec![
            sig_raw("military_flight", "a", json!({"aircraftType": "fighter"})),
            SignalEvidence {
                country: Some("US".into()),
                ..sig("military_flight", Some("US"), "b")
            },
            SignalEvidence {
                country: Some("RU".into()),
                ..sig("military_flight", Some("RU"), "c")
            },
            SignalEvidence {
                country: Some("CN".into()),
                ..sig("military_flight", Some("CN"), "d")
            },
        ];
        let title = MilitaryAdapter::new().generate_title(&signals, None, None);
        // Truncates at first two unique countries → "US/RU" (first
        // signal had no country, then US, then RU; CN dropped).
        assert!(title.contains("US/RU"), "got: {title}");
    }

    // ── escalation ─────────────────────────────────────────

    #[test]
    fn escalation_constants_match_source() {
        let a = EscalationAdapter::new();
        assert_eq!(a.domain(), CorrelationDomain::Escalation);
        assert_eq!(a.label(), "Escalation Monitor");
        assert_eq!(a.cluster_mode(), ClusterMode::Country);
        assert_eq!(a.spatial_radius_km(), 0.0);
        assert_eq!(a.time_window_hours(), 48);
        assert_eq!(a.threshold(), 20);
        let sum: f64 = a.weights().values().sum();
        assert!((sum - 1.0).abs() < 1e-9);
    }

    #[test]
    fn escalation_title_combines_present_types() {
        let signals = vec![
            sig("conflict_event", Some("UA"), "Kyiv strike"),
            sig("escalation_outage", Some("UA"), "Comms outage"),
            sig("news_severity", Some("UA"), "Headline"),
        ];
        let title = EscalationAdapter::new().generate_title(&signals, Some("UA"), None);
        assert_eq!(
            title,
            "conflict + comms disruption + news escalation \u{2014} UA"
        );
    }

    #[test]
    fn escalation_title_subset_only_lists_present() {
        let signals = vec![sig("conflict_event", Some("UA"), "x")];
        let title = EscalationAdapter::new().generate_title(&signals, Some("UA"), None);
        assert_eq!(title, "conflict \u{2014} UA");
    }

    #[test]
    fn escalation_title_fallback_when_no_known_types() {
        let signals = vec![sig("other", Some("UA"), "x")];
        let title = EscalationAdapter::new().generate_title(&signals, Some("UA"), None);
        assert_eq!(title, "Escalation signals \u{2014} UA");
    }

    // ── economic ───────────────────────────────────────────

    #[test]
    fn economic_constants_match_source() {
        let a = EconomicAdapter::new();
        assert_eq!(a.domain(), CorrelationDomain::Economic);
        assert_eq!(a.label(), "Economic Warfare");
        assert_eq!(a.cluster_mode(), ClusterMode::Entity);
        assert_eq!(a.time_window_hours(), 24);
        let sum: f64 = a.weights().values().sum();
        assert!((sum - 1.0).abs() < 1e-9);
        // Spot-check the commodity symbol set + threshold.
        assert!(COMMODITY_SYMBOLS.contains("CL=F"));
        assert!(COMMODITY_SYMBOLS.contains("BTC-USD"));
        assert!(!COMMODITY_SYMBOLS.contains("AAPL"));
        assert!((SIGNIFICANT_CHANGE_PCT - 1.5).abs() < f64::EPSILON);
    }

    #[test]
    fn economic_sanctions_plus_market_uses_extracted_country() {
        let signals = vec![
            sig("sanctions_news", None, "US announces new sanctions on Iran"),
            sig("market_move", None, "Brent +3.2%"),
        ];
        let title = EconomicAdapter::new().generate_title(&signals, None, Some("oil"));
        assert!(
            title.starts_with("Sanctions tightening:")
                && (title.contains("Iran")
                    || title.contains("US")
                    || title.contains("United States")),
            "got: {title}"
        );
    }

    #[test]
    fn economic_market_only_uses_display_or_symbol() {
        let signals = vec![
            sig_raw(
                "market_move",
                "Brent +3.2%",
                json!({"display": "Brent", "symbol": "BZ=F"}),
            ),
            sig_raw(
                "commodity_spike",
                "WTI +2.5%",
                json!({"display": "WTI", "symbol": "CL=F"}),
            ),
        ];
        let title = EconomicAdapter::new().generate_title(&signals, None, Some("oil"));
        assert_eq!(title, "Market disruption: Brent/WTI");
    }

    #[test]
    fn economic_fallback_when_only_generic_entity_key() {
        let signals = vec![sig("other", None, "some headline")];
        let title = EconomicAdapter::new().generate_title(&signals, None, Some("sanctions"));
        // GENERIC key → display_entity returns empty → fallback.
        assert_eq!(title, "Economic convergence detected");
    }

    #[test]
    fn economic_fallback_capitalises_non_generic_entity() {
        let signals = vec![sig("other", None, "some headline")];
        let title = EconomicAdapter::new().generate_title(&signals, None, Some("opec"));
        assert_eq!(title, "Economic convergence: Opec");
    }

    #[test]
    fn known_entities_regex_matches_iran_at_word_boundary() {
        assert!(KNOWN_ENTITIES_REGEX.is_match("US sanctions Iran on Tuesday"));
        // word boundary: don't match "Iranian" as "Iran".
        // Source regex uses `\b…\b(?![A-Za-z])` — Rust regex
        // crate's `\b` is identical, so "Iranian" does NOT match
        // "Iran" because the next char after "Iran" is alphanumeric.
        assert!(!KNOWN_ENTITIES_REGEX.is_match("Iranian foreign minister"));
    }

    #[test]
    fn sanctions_regex_hits_canonical_terms() {
        // The source regex anchors with `\b…\b` — "sanctions"
        // (plural) does NOT match `\bsanction\b` because the word
        // continues. Use the singular forms documented in
        // `economic.ts:11`. Inflected forms like "retaliat" /
        // "decoupl" / "manipulat" are stems with `\b` followed by
        // additional letters → also won't match in isolation; use
        // a labelled phrase instead.
        for term in [
            "sanction",
            "tariff",
            "embargo",
            "trade war",
            "rare earth",
            "supply chain",
            "petrodollar",
            "blacklist",
            "opec",
        ] {
            assert!(
                SANCTIONS_REGEX.is_match(term),
                "expected SANCTIONS_REGEX to match '{term}'"
            );
        }
    }

    #[test]
    fn never_match_helper_truly_never_matches() {
        let r = never_match();
        for s in ["", "a", "anything", "[a&&b]", "sanction"] {
            assert!(!r.is_match(s), "never_match must reject '{s}'");
        }
    }

    // ── disaster ───────────────────────────────────────────

    #[test]
    fn disaster_constants_match_source() {
        let a = DisasterAdapter::new();
        assert_eq!(a.domain(), CorrelationDomain::Disaster);
        assert_eq!(a.label(), "Disaster Cascade");
        assert_eq!(a.cluster_mode(), ClusterMode::Geographic);
        assert!((a.spatial_radius_km() - 500.0).abs() < f64::EPSILON);
        assert_eq!(a.time_window_hours(), 96);
        let sum: f64 = a.weights().values().sum();
        assert!((sum - 1.0).abs() < 1e-9);
    }

    #[test]
    fn disaster_seismic_only_uses_max_magnitude() {
        let signals = vec![
            sig_raw(
                "earthquake",
                "M5.2 \u{2014} 30km E of Tehran",
                json!({"magnitude": 5.2}),
            ),
            sig_raw(
                "earthquake",
                "M6.1 \u{2014} 15km N of Karaj",
                json!({"magnitude": 6.1}),
            ),
        ];
        let title = DisasterAdapter::new().generate_title(&signals, None, None);
        assert!(title.contains("M6.1 seismic"));
        // Place from FIRST earthquake's label, after the em-dash.
        assert!(title.contains("30km E of Tehran"));
    }

    #[test]
    fn disaster_combined_seismic_and_infra_outage() {
        let signals = vec![
            sig_raw(
                "earthquake",
                "M4.5 \u{2014} Coastal Tehran",
                json!({"magnitude": 4.5}),
            ),
            sig("infra_outage", None, "Power grid stress"),
        ];
        let title = DisasterAdapter::new().generate_title(&signals, None, None);
        assert!(title.contains("M4.5 seismic + infra disruption"));
    }

    #[test]
    fn disaster_fallback_when_no_matching_types() {
        let signals = vec![sig("other", None, "some headline")];
        let title = DisasterAdapter::new().generate_title(&signals, None, None);
        assert_eq!(title, "Disaster convergence detected");
    }

    // ── helpers ────────────────────────────────────────────

    #[test]
    fn first_two_unique_drops_repeats_and_caps_at_two() {
        let signals = vec![
            sig("x", Some("US"), ""),
            sig("x", Some("US"), ""),
            sig("x", Some("RU"), ""),
            sig("x", Some("CN"), ""),
        ];
        assert_eq!(first_two_unique_countries(&signals), vec!["US", "RU"]);
    }
}
