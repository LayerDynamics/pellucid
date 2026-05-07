//! Cross-domain correlation engine — port of
//! `worldmonitor/src/services/correlation-engine/{engine,types}.ts`.
//!
//! Surfaces the four-domain `Correlator` trait
//! (`Military` / `Escalation` / `Economic` / `Disaster`), the
//! grid-union-find proximity clustering, country / entity-keyword
//! clustering, weighted-with-diversity-bonus scoring, trend
//! detection across cycles, and the [`ConvergenceCard`] output
//! shape consumed by `correlation/v1/run`.
//!
//! The signal-collector / domain-adapter side is split out per
//! caller: each adapter's `collect_signals` is implemented as a
//! pure function that takes a typed cache snapshot and emits
//! `Vec<SignalEvidence>`. The engine here doesn't care where the
//! signals came from — it just clusters, scores, and renders.

use std::collections::{BTreeSet, HashMap};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

// ============================================================================
// Public types — port of `correlation-engine/types.ts`
// ============================================================================

/// Domain bucket for a [`Correlator`] / [`ConvergenceCard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CorrelationDomain {
    Military,
    Escalation,
    Economic,
    Disaster,
}

impl CorrelationDomain {
    /// Stable lower-snake string used in card ids and the LLM
    /// cache key. Matches `engine.ts:349, 386` exactly.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Military => "military",
            Self::Escalation => "escalation",
            Self::Economic => "economic",
            Self::Disaster => "disaster",
        }
    }
}

/// Trend tag attached to a [`ConvergenceCard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum TrendDirection {
    Escalating,
    Stable,
    DeEscalating,
}

/// Per-signal evidence row that feeds the engine. Mirrors
/// `SignalEvidence` (`types.ts:9-19`) field-for-field.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalEvidence {
    /// Stable signal type tag (e.g. `military_flight`,
    /// `conflict_event`). Used by the adapter's weight map +
    /// diversity bonus.
    #[serde(rename = "type")]
    pub signal_type: String,
    pub source: String,
    /// 0..=100 severity. Higher is worse.
    pub severity: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lat: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lon: Option<f64>,
    /// ISO-2 country code, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    /// UNIX millisecond epoch. The engine doesn't filter on time
    /// — adapters apply their own `time_window` before emitting.
    pub timestamp: i64,
    pub label: String,
    /// Untyped passthrough for adapter-specific raw payload. Kept
    /// as `serde_json::Value` so it round-trips across the IPC
    /// surface without per-adapter polymorphism.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub raw_data: Option<Value>,
}

/// One row emitted by the engine after clustering + scoring.
/// Mirrors `ConvergenceCard` (`types.ts:21-32`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConvergenceCard {
    pub id: String,
    pub domain: CorrelationDomain,
    pub title: String,
    /// `[0, 100]` rounded composite — sum of (max severity per
    /// type) × weight, plus a diversity bonus for multi-type
    /// clusters, capped at 100.
    pub score: u32,
    pub signals: Vec<SignalEvidence>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<CardLocation>,
    pub countries: Vec<String>,
    pub trend: TrendDirection,
    pub timestamp: DateTime<Utc>,
    /// Optional LLM narrative — populated asynchronously after
    /// the card has been emitted (via the intelligence service
    /// `deductSituation` path on the JS side). Always `None` on
    /// first emission; the host fills this when the call returns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assessment: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardLocation {
    pub lat: f64,
    pub lon: f64,
    /// Display label: country code for country/entity clusters,
    /// `"<lat>,<lon>"` for proximity clusters.
    pub label: String,
}

/// Grouping mode the engine uses for an adapter's signals. Mirrors
/// `types.ts:34`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClusterMode {
    Geographic,
    Country,
    Entity,
}

/// Per-cycle cluster state captured for trend detection on the
/// next cycle. Mirrors `types.ts:48-56`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClusterState {
    /// Stable identity for the cluster — country code for
    /// `Country` mode, entity-keyword for `Entity` mode, or
    /// `"<lat:.1>,<lon:.1>"` for `Geographic`.
    pub key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub centroid_lat: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub centroid_lon: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub country: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entity_key: Option<String>,
    pub score: u32,
    pub timestamp: i64,
}

/// Domain adapter — pluggable per-correlator config + the title
/// renderer the engine calls after scoring. Concrete impls live
/// in [`crate::adapters`] (one per domain).
pub trait Correlator: Send + Sync {
    fn domain(&self) -> CorrelationDomain;
    /// Human-readable label rendered into panel headers.
    fn label(&self) -> &str;
    fn cluster_mode(&self) -> ClusterMode;
    /// Spatial radius in km — only meaningful for
    /// [`ClusterMode::Geographic`]. Pass 0 otherwise.
    fn spatial_radius_km(&self) -> f64;
    /// Time window in hours; the engine doesn't filter on this
    /// directly — collectors apply it. Stored here so the spec
    /// stays self-documenting.
    fn time_window_hours(&self) -> u32;
    /// Minimum score to emit a card. Below this threshold the
    /// cluster is dropped from the output.
    fn threshold(&self) -> u32;
    /// Per-signal-type weight map. Each weight in `[0, 1]`; sum
    /// SHOULD be ≤ 1.0 (renormalize when adding new signal types).
    fn weights(&self) -> &HashMap<String, f64>;
    /// Render a cluster's title. Receives the cluster signals and
    /// optional country / entity-key hints.
    fn generate_title(
        &self,
        signals: &[SignalEvidence],
        country: Option<&str>,
        entity_key: Option<&str>,
    ) -> String;
}

// ============================================================================
// Engine
// ============================================================================

/// The clustering / scoring / trend pipeline. Holds the
/// previous-cycle [`ClusterState`] per domain so trend can be
/// computed without callers tracking that themselves.
#[derive(Debug, Default)]
pub struct CorrelationEngine {
    previous_clusters: HashMap<CorrelationDomain, Vec<ClusterState>>,
}

impl CorrelationEngine {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Run the pipeline for one adapter. Returns the cards
    /// (sorted by score desc) and persists the cluster state for
    /// next-cycle trend detection.
    pub fn run<A: Correlator>(
        &mut self,
        adapter: &A,
        signals: Vec<SignalEvidence>,
        now: DateTime<Utc>,
    ) -> Vec<ConvergenceCard> {
        let clusters = self.cluster_signals(adapter, signals);
        let scored: Vec<ScoredCluster> = clusters
            .into_iter()
            .map(|c| score_cluster(&c, adapter))
            .collect();
        let filtered: Vec<ScoredCluster> = scored
            .into_iter()
            .filter(|s| s.score >= adapter.threshold())
            .collect();
        let with_trend = self.apply_trends(adapter, filtered);

        // Save cluster state for next cycle.
        let next_state: Vec<ClusterState> = with_trend.iter().map(|c| c.state.clone()).collect();
        self.previous_clusters.insert(adapter.domain(), next_state);

        let mut cards: Vec<ConvergenceCard> = with_trend
            .into_iter()
            .map(|c| build_card(adapter, c, now))
            .collect();
        cards.sort_by_key(|c| std::cmp::Reverse(c.score));
        cards
    }

    /// Drop persisted state — used at startup or after an explicit
    /// reset.
    pub fn reset(&mut self) {
        self.previous_clusters.clear();
    }

    fn cluster_signals<A: Correlator>(
        &self,
        adapter: &A,
        signals: Vec<SignalEvidence>,
    ) -> Vec<SignalCluster> {
        if signals.is_empty() {
            return Vec::new();
        }
        match adapter.cluster_mode() {
            ClusterMode::Country => cluster_by_country(signals),
            ClusterMode::Entity => cluster_by_entity(signals),
            ClusterMode::Geographic => cluster_by_proximity(signals, adapter.spatial_radius_km()),
        }
    }

    fn apply_trends<A: Correlator>(
        &self,
        adapter: &A,
        scored: Vec<ScoredCluster>,
    ) -> Vec<ScoredCluster> {
        let half_radius = adapter.spatial_radius_km() / 2.0;
        let previous = self
            .previous_clusters
            .get(&adapter.domain())
            .cloned()
            .unwrap_or_default();
        scored
            .into_iter()
            .map(|mut sc| {
                let matched = previous.iter().find(|prev| {
                    if let (Some(c), Some(pc)) = (sc.state.country.as_ref(), prev.country.as_ref())
                    {
                        return c == pc;
                    }
                    if let (Some(e), Some(pe)) =
                        (sc.state.entity_key.as_ref(), prev.entity_key.as_ref())
                    {
                        return e == pe;
                    }
                    if let (Some(la), Some(lo), Some(pla), Some(plo)) = (
                        sc.centroid_lat,
                        sc.centroid_lon,
                        prev.centroid_lat,
                        prev.centroid_lon,
                    ) {
                        return haversine_km(la, lo, pla, plo) <= half_radius;
                    }
                    false
                });
                if let Some(prev) = matched {
                    let delta = i64::from(sc.score) - i64::from(prev.score);
                    sc.trend = if delta > 5 {
                        TrendDirection::Escalating
                    } else if delta < -5 {
                        TrendDirection::DeEscalating
                    } else {
                        TrendDirection::Stable
                    };
                }
                sc
            })
            .collect()
    }
}

// ============================================================================
// Internal cluster types
// ============================================================================

#[derive(Debug, Clone)]
struct SignalCluster {
    signals: Vec<SignalEvidence>,
    country: Option<String>,
    entity_key: Option<String>,
}

#[derive(Debug, Clone)]
struct ScoredCluster {
    cluster: SignalCluster,
    score: u32,
    countries: Vec<String>,
    centroid_lat: Option<f64>,
    centroid_lon: Option<f64>,
    state: ClusterState,
    trend: TrendDirection,
}

// ============================================================================
// Clustering — port of `engine.ts:111-238`
// ============================================================================

/// Group signals by `country`. Drops any signal without a country
/// code; clusters with fewer than 2 signals are discarded
/// (mirrors `engine.ts:121`).
fn cluster_by_country(signals: Vec<SignalEvidence>) -> Vec<SignalCluster> {
    let mut by_country: HashMap<String, Vec<SignalEvidence>> = HashMap::new();
    for s in signals {
        if let Some(c) = s.country.clone() {
            by_country.entry(c).or_default().push(s);
        }
    }
    let mut out = Vec::new();
    // BTreeSet for deterministic iteration so test outputs are
    // stable across platforms.
    let keys: BTreeSet<String> = by_country.keys().cloned().collect();
    for c in keys {
        let sigs = by_country.remove(&c).unwrap_or_default();
        if sigs.len() < 2 {
            continue;
        }
        out.push(SignalCluster {
            signals: sigs,
            country: Some(c),
            entity_key: None,
        });
    }
    out
}

const COMPOUND_PATTERNS: &[&str] = &[
    "supply chain",
    "rare earth",
    "central bank",
    "interest rate",
    "trade war",
    "oil price",
    "gas price",
    "federal reserve",
];

/// Single-token entity-keyword set — mirrors `engine.ts:134-139`.
const SINGLE_KEYS: &[&str] = &[
    "oil",
    "gas",
    "sanctions",
    "trade",
    "tariff",
    "commodity",
    "currency",
    "energy",
    "wheat",
    "crude",
    "gold",
    "silver",
    "copper",
    "bitcoin",
    "crypto",
    "inflation",
    "embargo",
    "opec",
    "semiconductor",
    "dollar",
    "yuan",
    "euro",
];

/// Group signals by entity keyword detected in their `label`.
/// Compound patterns checked first to avoid false positives from
/// ambiguous single words. Signals with no match drop. Clusters
/// with fewer than 2 signals are discarded.
fn cluster_by_entity(signals: Vec<SignalEvidence>) -> Vec<SignalCluster> {
    let mut by_key: HashMap<String, Vec<SignalEvidence>> = HashMap::new();
    for s in signals {
        let lower = s.label.to_lowercase();
        let mut matched: Option<&str> = None;
        for &p in COMPOUND_PATTERNS {
            if lower.contains(p) {
                matched = Some(p);
                break;
            }
        }
        if matched.is_none() {
            for word in lower.split(|c: char| !c.is_alphanumeric()) {
                if SINGLE_KEYS.contains(&word) {
                    matched = Some(word);
                    break;
                }
            }
        }
        let Some(key) = matched else { continue };
        by_key.entry(key.to_string()).or_default().push(s);
    }
    let mut out = Vec::new();
    let keys: BTreeSet<String> = by_key.keys().cloned().collect();
    for k in keys {
        let sigs = by_key.remove(&k).unwrap_or_default();
        if sigs.len() < 2 {
            continue;
        }
        out.push(SignalCluster {
            signals: sigs,
            country: None,
            entity_key: Some(k),
        });
    }
    out
}

/// Grid-based proximity clustering with union-find. Port of
/// `engine.ts:165-238`. Signals without `(lat, lon)` are
/// dropped. Output cluster size ≥ 2.
fn cluster_by_proximity(signals: Vec<SignalEvidence>, radius_km: f64) -> Vec<SignalCluster> {
    if radius_km <= 0.0 {
        return Vec::new();
    }
    const DEG_PER_KM_LAT: f64 = 1.0 / 111.0;
    let cell_size_lat = radius_km * DEG_PER_KM_LAT;

    let n = signals.len();
    let mut parent: Vec<usize> = (0..n).collect();

    // Union-find with path halving.
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    fn union(parent: &mut [usize], a: usize, b: usize) {
        let ra = find(parent, a);
        let rb = find(parent, b);
        if ra != rb {
            parent[ra] = rb;
        }
    }

    // Index valid signals into a spatial grid keyed by `(row, col)`.
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    let mut valid: Vec<usize> = Vec::new();
    for (i, s) in signals.iter().enumerate() {
        let (Some(lat), Some(lon)) = (s.lat, s.lon) else {
            continue;
        };
        valid.push(i);
        let cell_row = (lat / cell_size_lat).floor() as i64;
        let cos_lat = (lat * std::f64::consts::PI / 180.0).cos();
        let cell_size_lon = if cos_lat > 0.01 {
            cell_size_lat / cos_lat
        } else {
            cell_size_lat
        };
        let cell_col = (lon / cell_size_lon).floor() as i64;
        grid.entry((cell_row, cell_col)).or_default().push(i);
    }

    // 3×3 neighbourhood union for each cell.
    let cells: Vec<((i64, i64), Vec<usize>)> = grid.iter().map(|(k, v)| (*k, v.clone())).collect();
    for ((row, col), indices) in &cells {
        for dr in -1_i64..=1 {
            for dc in -1_i64..=1 {
                let Some(neighbours) = grid.get(&(row + dr, col + dc)) else {
                    continue;
                };
                for &i in indices {
                    let (Some(lat_i), Some(lon_i)) = (signals[i].lat, signals[i].lon) else {
                        continue;
                    };
                    for &j in neighbours {
                        if i >= j {
                            continue;
                        }
                        let (Some(lat_j), Some(lon_j)) = (signals[j].lat, signals[j].lon) else {
                            continue;
                        };
                        if haversine_km(lat_i, lon_i, lat_j, lon_j) <= radius_km {
                            union(&mut parent, i, j);
                        }
                    }
                }
            }
        }
    }

    // Collect clusters from the union-find roots.
    let mut groups: HashMap<usize, Vec<SignalEvidence>> = HashMap::new();
    for i in valid {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(signals[i].clone());
    }
    // Deterministic ordering by smallest index in the group.
    let mut sorted_roots: Vec<usize> = groups.keys().copied().collect();
    sorted_roots.sort_unstable();

    let mut out = Vec::new();
    for root in sorted_roots {
        let sigs = groups.remove(&root).unwrap_or_default();
        if sigs.len() < 2 {
            continue;
        }
        out.push(SignalCluster {
            signals: sigs,
            country: None,
            entity_key: None,
        });
    }
    out
}

// ============================================================================
// Scoring — port of `engine.ts:242-300`
// ============================================================================

fn score_cluster<A: Correlator>(cluster: &SignalCluster, adapter: &A) -> ScoredCluster {
    // Aggregate max severity per signal type.
    let mut per_type: HashMap<String, u32> = HashMap::new();
    for s in &cluster.signals {
        let entry = per_type.entry(s.signal_type.clone()).or_insert(0);
        if s.severity > *entry {
            *entry = s.severity;
        }
    }
    let weights = adapter.weights();
    let mut weighted_sum = 0.0_f64;
    for (ty, sev) in &per_type {
        let w = weights.get(ty).copied().unwrap_or(0.0);
        weighted_sum += f64::from(*sev) * w;
    }
    let unique_types = per_type.len() as i64;
    // Diversity bonus capped at 30: 12 per unique type beyond the
    // first two.
    let diversity_bonus = ((unique_types - 2).max(0) * 12).min(30) as f64;
    let final_score = (weighted_sum + diversity_bonus).clamp(0.0, 100.0);
    let final_score_u = final_score.round() as u32;

    // Centroid for geographic clusters. Longitude uses a circular
    // mean (atan2 of unit-vector components) so the antimeridian
    // doesn't fold the centroid back toward 0°.
    let geo: Vec<(f64, f64)> = cluster
        .signals
        .iter()
        .filter_map(|s| match (s.lat, s.lon) {
            (Some(la), Some(lo)) => Some((la, lo)),
            _ => None,
        })
        .collect();
    let (centroid_lat, centroid_lon) = if geo.is_empty() {
        (None, None)
    } else {
        let mean_lat = geo.iter().map(|(la, _)| *la).sum::<f64>() / geo.len() as f64;
        let to_rad = std::f64::consts::PI / 180.0;
        let to_deg = 180.0 / std::f64::consts::PI;
        let (mut sin_sum, mut cos_sum) = (0.0_f64, 0.0_f64);
        for &(_, lo) in &geo {
            sin_sum += (lo * to_rad).sin();
            cos_sum += (lo * to_rad).cos();
        }
        (Some(mean_lat), Some(sin_sum.atan2(cos_sum) * to_deg))
    };

    let mut country_set: BTreeSet<String> = BTreeSet::new();
    for s in &cluster.signals {
        if let Some(c) = s.country.clone() {
            country_set.insert(c);
        }
    }
    let countries: Vec<String> = country_set.into_iter().collect();

    let key = cluster
        .country
        .clone()
        .or_else(|| cluster.entity_key.clone())
        .unwrap_or_else(|| match (centroid_lat, centroid_lon) {
            (Some(la), Some(lo)) => format!("{la:.1},{lo:.1}"),
            _ => "unknown".to_string(),
        });

    let state = ClusterState {
        key,
        centroid_lat,
        centroid_lon,
        country: cluster.country.clone(),
        entity_key: cluster.entity_key.clone(),
        score: final_score_u,
        timestamp: chrono::Utc::now().timestamp_millis(),
    };

    ScoredCluster {
        cluster: cluster.clone(),
        score: final_score_u,
        countries,
        centroid_lat,
        centroid_lon,
        state,
        trend: TrendDirection::Stable,
    }
}

// ============================================================================
// Card construction — port of `engine.ts:336-359`
// ============================================================================

fn build_card<A: Correlator>(
    adapter: &A,
    sc: ScoredCluster,
    now: DateTime<Utc>,
) -> ConvergenceCard {
    let title = adapter.generate_title(
        &sc.cluster.signals,
        sc.cluster.country.as_deref(),
        sc.cluster.entity_key.as_deref(),
    );
    let location = match (sc.centroid_lat, sc.centroid_lon) {
        (Some(la), Some(lo)) => Some(CardLocation {
            lat: la,
            lon: lo,
            label: sc.state.key.clone(),
        }),
        _ => None,
    };
    ConvergenceCard {
        id: format!("{}:{}", adapter.domain().as_str(), sc.state.key),
        domain: adapter.domain(),
        title,
        score: sc.score,
        signals: sc.cluster.signals,
        location,
        countries: sc.countries,
        trend: sc.trend,
        timestamp: now,
        assessment: None,
    }
}

// ============================================================================
// Haversine — great-circle distance in km
// ============================================================================

const EARTH_RADIUS_KM: f64 = 6371.0;

fn haversine_km(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    let to_rad = std::f64::consts::PI / 180.0;
    let phi1 = lat1 * to_rad;
    let phi2 = lat2 * to_rad;
    let dphi = (lat2 - lat1) * to_rad;
    let dlambda = (lon2 - lon1) * to_rad;
    let a = (dphi / 2.0).sin().powi(2) + phi1.cos() * phi2.cos() * (dlambda / 2.0).sin().powi(2);
    let c = 2.0 * a.sqrt().atan2((1.0 - a).sqrt());
    EARTH_RADIUS_KM * c
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 5, 4, 12, 0, 0).unwrap()
    }

    fn signal(
        ty: &str,
        sev: u32,
        country: Option<&str>,
        lat: Option<f64>,
        lon: Option<f64>,
        label: &str,
    ) -> SignalEvidence {
        SignalEvidence {
            signal_type: ty.into(),
            source: "test".into(),
            severity: sev,
            lat,
            lon,
            country: country.map(str::to_string),
            timestamp: 0,
            label: label.into(),
            raw_data: None,
        }
    }

    /// Test adapter — geographic clustering of two signal types.
    struct MilTestAdapter {
        weights: HashMap<String, f64>,
    }
    impl MilTestAdapter {
        fn new() -> Self {
            let mut w = HashMap::new();
            w.insert("military_flight".into(), 0.5);
            w.insert("military_vessel".into(), 0.5);
            Self { weights: w }
        }
    }
    impl Correlator for MilTestAdapter {
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
        fn generate_title(
            &self,
            _signals: &[SignalEvidence],
            _country: Option<&str>,
            _entity: Option<&str>,
        ) -> String {
            "test title".into()
        }
    }

    /// Country-mode test adapter.
    struct CountryAdapter {
        weights: HashMap<String, f64>,
    }
    impl CountryAdapter {
        fn new() -> Self {
            let mut w = HashMap::new();
            w.insert("conflict_event".into(), 1.0);
            Self { weights: w }
        }
    }
    impl Correlator for CountryAdapter {
        fn domain(&self) -> CorrelationDomain {
            CorrelationDomain::Escalation
        }
        fn label(&self) -> &str {
            "Escalation"
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
        fn generate_title(
            &self,
            _: &[SignalEvidence],
            country: Option<&str>,
            _: Option<&str>,
        ) -> String {
            format!("Escalation in {}", country.unwrap_or("?"))
        }
    }

    /// Entity-mode test adapter.
    struct EntityAdapter {
        weights: HashMap<String, f64>,
    }
    impl EntityAdapter {
        fn new() -> Self {
            let mut w = HashMap::new();
            w.insert("commodity_shock".into(), 1.0);
            Self { weights: w }
        }
    }
    impl Correlator for EntityAdapter {
        fn domain(&self) -> CorrelationDomain {
            CorrelationDomain::Economic
        }
        fn label(&self) -> &str {
            "Economic"
        }
        fn cluster_mode(&self) -> ClusterMode {
            ClusterMode::Entity
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
        fn generate_title(
            &self,
            _: &[SignalEvidence],
            _: Option<&str>,
            ek: Option<&str>,
        ) -> String {
            format!("Economic: {}", ek.unwrap_or("?"))
        }
    }

    // ── haversine ────────────────────────────────────────────

    #[test]
    fn haversine_zero_distance_is_zero() {
        assert!(haversine_km(35.7, 51.4, 35.7, 51.4) < 1e-9);
    }

    #[test]
    fn haversine_known_distance_in_range() {
        // Tehran (35.7, 51.4) → Tel Aviv (32.08, 34.78) ~ 1500 km.
        let d = haversine_km(35.7, 51.4, 32.08, 34.78);
        assert!(d > 1400.0 && d < 1600.0, "got {d}");
    }

    // ── proximity clustering ────────────────────────────────

    #[test]
    fn proximity_clusters_two_close_signals() {
        let signals = vec![
            signal(
                "military_flight",
                70,
                Some("IR"),
                Some(35.7),
                Some(51.4),
                "Tehran flight",
            ),
            signal(
                "military_vessel",
                70,
                Some("IR"),
                Some(35.71),
                Some(51.41),
                "Caspian vessel",
            ),
        ];
        let mut engine = CorrelationEngine::new();
        let adapter = MilTestAdapter::new();
        let cards = engine.run(&adapter, signals, now());
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].domain, CorrelationDomain::Military);
        assert!(cards[0].score >= 20);
        assert_eq!(cards[0].signals.len(), 2);
    }

    #[test]
    fn proximity_does_not_cluster_far_signals() {
        let signals = vec![
            signal(
                "military_flight",
                70,
                Some("IR"),
                Some(35.7),
                Some(51.4),
                "Tehran",
            ),
            signal(
                "military_flight",
                70,
                Some("US"),
                Some(38.9),
                Some(-77.0),
                "DC",
            ),
        ];
        let mut engine = CorrelationEngine::new();
        let adapter = MilTestAdapter::new();
        let cards = engine.run(&adapter, signals, now());
        assert!(
            cards.is_empty(),
            "two far-apart singletons must not produce a cluster"
        );
    }

    #[test]
    fn proximity_drops_signals_without_coordinates() {
        let signals = vec![
            signal(
                "military_flight",
                70,
                Some("IR"),
                Some(35.7),
                Some(51.4),
                "geo",
            ),
            signal("military_flight", 70, Some("IR"), None, None, "no-geo"),
        ];
        let mut engine = CorrelationEngine::new();
        let adapter = MilTestAdapter::new();
        let cards = engine.run(&adapter, signals, now());
        // Single geolocated signal cannot form a cluster.
        assert!(cards.is_empty());
    }

    // ── country clustering ──────────────────────────────────

    #[test]
    fn country_mode_groups_by_country_code() {
        let signals = vec![
            signal("conflict_event", 70, Some("UA"), None, None, "kyiv"),
            signal("conflict_event", 50, Some("UA"), None, None, "kharkiv"),
            signal("conflict_event", 60, Some("RU"), None, None, "moscow"),
        ];
        let mut engine = CorrelationEngine::new();
        let adapter = CountryAdapter::new();
        let cards = engine.run(&adapter, signals, now());
        // UA has 2 → clustered. RU has 1 → dropped (size < 2).
        assert_eq!(cards.len(), 1);
        assert!(cards[0].countries.contains(&"UA".to_string()));
    }

    // ── entity clustering ───────────────────────────────────

    #[test]
    fn entity_mode_groups_by_keyword() {
        let signals = vec![
            signal(
                "commodity_shock",
                70,
                None,
                None,
                None,
                "oil prices spike on supply chain disruption",
            ),
            signal(
                "commodity_shock",
                60,
                None,
                None,
                None,
                "Brent oil futures rally",
            ),
            // Different topic: copper. Single signal → dropped.
            signal(
                "commodity_shock",
                50,
                None,
                None,
                None,
                "copper inventories down",
            ),
        ];
        let mut engine = CorrelationEngine::new();
        let adapter = EntityAdapter::new();
        let cards = engine.run(&adapter, signals, now());
        // The compound "supply chain" matches FIRST per the JS
        // semantics, so both signal-1 and signal-2 (which has
        // "oil") DO end up in different keyword buckets:
        // signal-1 → "supply chain", signal-2 → "oil". Each
        // bucket is size 1 → dropped. So we expect 0 cards.
        // This test pins that semantic.
        assert!(cards.is_empty());
    }

    #[test]
    fn entity_mode_clusters_two_oil_signals() {
        // Both labels MUST avoid the `oil price` / `gas price` /
        // `supply chain` etc. compound patterns so they fall through
        // to the single-key `oil` bucket and cluster together.
        let signals = vec![
            signal(
                "commodity_shock",
                70,
                None,
                None,
                None,
                "oil exports surge after embargo lift",
            ),
            signal(
                "commodity_shock",
                60,
                None,
                None,
                None,
                "Brent oil futures rally on news",
            ),
        ];
        let mut engine = CorrelationEngine::new();
        let adapter = EntityAdapter::new();
        let cards = engine.run(&adapter, signals, now());
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].domain, CorrelationDomain::Economic);
        assert_eq!(cards[0].id, "economic:oil");
    }

    // ── scoring ─────────────────────────────────────────────

    #[test]
    fn diversity_bonus_kicks_in_above_two_unique_types() {
        // Two-type cluster: bonus = 0.
        let two_type = vec![
            signal("military_flight", 80, None, Some(35.7), Some(51.4), "a"),
            signal("military_vessel", 80, None, Some(35.71), Some(51.41), "b"),
        ];
        // Three-type cluster: bonus = 12.
        let mut adapter = MilTestAdapter::new();
        adapter.weights.insert("ais_gap".into(), 0.5);
        let three_type = vec![
            signal("military_flight", 80, None, Some(35.7), Some(51.4), "a"),
            signal("military_vessel", 80, None, Some(35.71), Some(51.41), "b"),
            signal("ais_gap", 80, None, Some(35.72), Some(51.42), "c"),
        ];
        let mut e1 = CorrelationEngine::new();
        let mut e2 = CorrelationEngine::new();
        let two = e1.run(&MilTestAdapter::new(), two_type, now());
        let three = e2.run(&adapter, three_type, now());
        assert!(three[0].score > two[0].score);
    }

    #[test]
    fn score_is_capped_at_one_hundred() {
        // Three signal types at max severity with weights summing
        // to 1.5 — without the cap, the weighted sum would be
        // 80*1.5 + 12 = 132. The cap clamps to 100.
        let mut weights = HashMap::new();
        weights.insert("a".into(), 0.5);
        weights.insert("b".into(), 0.5);
        weights.insert("c".into(), 0.5);
        struct WideAdapter {
            weights: HashMap<String, f64>,
        }
        impl Correlator for WideAdapter {
            fn domain(&self) -> CorrelationDomain {
                CorrelationDomain::Disaster
            }
            fn label(&self) -> &str {
                "x"
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
            fn generate_title(
                &self,
                _: &[SignalEvidence],
                _: Option<&str>,
                _: Option<&str>,
            ) -> String {
                "x".into()
            }
        }
        let signals = vec![
            signal("a", 100, None, Some(0.0), Some(0.0), ""),
            signal("b", 100, None, Some(0.01), Some(0.01), ""),
            signal("c", 100, None, Some(0.02), Some(0.02), ""),
        ];
        let mut engine = CorrelationEngine::new();
        let cards = engine.run(&WideAdapter { weights }, signals, now());
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].score, 100);
    }

    // ── trend detection ─────────────────────────────────────

    #[test]
    fn trend_escalating_when_score_increases_more_than_five() {
        // Cycle 1: country UA, score X. Cycle 2: country UA,
        // score X+10 → trend is escalating.
        let mut engine = CorrelationEngine::new();
        let adapter = CountryAdapter::new();
        // Cycle 1 — two low-severity signals.
        let s1 = vec![
            signal("conflict_event", 30, Some("UA"), None, None, "a"),
            signal("conflict_event", 30, Some("UA"), None, None, "b"),
        ];
        let _ = engine.run(&adapter, s1, now());
        // Cycle 2 — same country, higher severities.
        let s2 = vec![
            signal("conflict_event", 80, Some("UA"), None, None, "c"),
            signal("conflict_event", 80, Some("UA"), None, None, "d"),
        ];
        let cards2 = engine.run(&adapter, s2, now());
        assert_eq!(cards2.len(), 1);
        assert_eq!(cards2[0].trend, TrendDirection::Escalating);
    }

    #[test]
    fn trend_de_escalating_when_score_drops() {
        let mut engine = CorrelationEngine::new();
        let adapter = CountryAdapter::new();
        let s1 = vec![
            signal("conflict_event", 90, Some("UA"), None, None, "a"),
            signal("conflict_event", 90, Some("UA"), None, None, "b"),
        ];
        let _ = engine.run(&adapter, s1, now());
        let s2 = vec![
            signal("conflict_event", 25, Some("UA"), None, None, "c"),
            signal("conflict_event", 25, Some("UA"), None, None, "d"),
        ];
        let cards2 = engine.run(&adapter, s2, now());
        assert_eq!(cards2.len(), 1);
        assert_eq!(cards2[0].trend, TrendDirection::DeEscalating);
    }

    #[test]
    fn trend_stable_when_score_within_five_points() {
        let mut engine = CorrelationEngine::new();
        let adapter = CountryAdapter::new();
        let s1 = vec![
            signal("conflict_event", 50, Some("UA"), None, None, "a"),
            signal("conflict_event", 50, Some("UA"), None, None, "b"),
        ];
        let _ = engine.run(&adapter, s1, now());
        let s2 = vec![
            signal("conflict_event", 52, Some("UA"), None, None, "c"),
            signal("conflict_event", 52, Some("UA"), None, None, "d"),
        ];
        let cards2 = engine.run(&adapter, s2, now());
        assert_eq!(cards2.len(), 1);
        assert_eq!(cards2[0].trend, TrendDirection::Stable);
    }

    #[test]
    fn trend_stable_when_no_previous_match() {
        let mut engine = CorrelationEngine::new();
        let adapter = CountryAdapter::new();
        let s = vec![
            signal("conflict_event", 70, Some("UA"), None, None, "a"),
            signal("conflict_event", 70, Some("UA"), None, None, "b"),
        ];
        let cards = engine.run(&adapter, s, now());
        // Cold start — default trend is Stable.
        assert_eq!(cards[0].trend, TrendDirection::Stable);
    }

    #[test]
    fn reset_clears_previous_clusters() {
        let mut engine = CorrelationEngine::new();
        let adapter = CountryAdapter::new();
        let s1 = vec![
            signal("conflict_event", 90, Some("UA"), None, None, "a"),
            signal("conflict_event", 90, Some("UA"), None, None, "b"),
        ];
        let _ = engine.run(&adapter, s1, now());
        engine.reset();
        let s2 = vec![
            signal("conflict_event", 25, Some("UA"), None, None, "c"),
            signal("conflict_event", 25, Some("UA"), None, None, "d"),
        ];
        let cards2 = engine.run(&adapter, s2, now());
        // Without prior state, trend defaults to Stable.
        assert_eq!(cards2[0].trend, TrendDirection::Stable);
    }

    // ── card output ─────────────────────────────────────────

    #[test]
    fn cards_sorted_by_score_descending() {
        let mut engine = CorrelationEngine::new();
        let adapter = CountryAdapter::new();
        let s = vec![
            signal("conflict_event", 30, Some("UA"), None, None, ""),
            signal("conflict_event", 30, Some("UA"), None, None, ""),
            signal("conflict_event", 90, Some("RU"), None, None, ""),
            signal("conflict_event", 90, Some("RU"), None, None, ""),
        ];
        let cards = engine.run(&adapter, s, now());
        assert_eq!(cards.len(), 2);
        assert!(cards[0].score >= cards[1].score);
        assert!(cards[0].countries.contains(&"RU".to_string()));
    }

    #[test]
    fn card_id_uses_domain_colon_key() {
        let mut engine = CorrelationEngine::new();
        let adapter = CountryAdapter::new();
        let s = vec![
            signal("conflict_event", 70, Some("UA"), None, None, ""),
            signal("conflict_event", 70, Some("UA"), None, None, ""),
        ];
        let cards = engine.run(&adapter, s, now());
        assert_eq!(cards[0].id, "escalation:UA");
    }

    #[test]
    fn below_threshold_clusters_are_dropped() {
        // Single signal type, low severity. Score = 30 * 0.5 = 15.
        // Below threshold of 20 → no card.
        let mut engine = CorrelationEngine::new();
        let adapter = CountryAdapter::new();
        let s = vec![
            signal("conflict_event", 15, Some("UA"), None, None, ""),
            signal("conflict_event", 15, Some("UA"), None, None, ""),
        ];
        let cards = engine.run(&adapter, s, now());
        // 15 * 1.0 = 15 + 0 diversity (1 type) = 15 < 20 threshold.
        assert!(cards.is_empty());
    }
}
