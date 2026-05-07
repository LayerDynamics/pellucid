//! Correlation-signal detectors — port of the eight detectors in
//! `worldmonitor/src/services/analysis-core.ts`:
//!
//! 1. `detect_pipeline_flow_drops`  — port of `detectPipelineFlowDrops`
//! 2. `detect_convergence`          — port of `detectConvergence`
//! 3. `detect_triangulation`        — port of `detectTriangulation`
//! 4. `detect_prediction_shifts`    — extracted from `analyzeCorrelationsCore`
//! 5. `detect_velocity_spikes`      — extracted from `analyzeCorrelationsCore`
//! 6. `detect_market_moves`         — extracted (`explained_market_move` ⊕ `silent_divergence`)
//! 7. `detect_flow_price_divergence`— extracted
//! 8. `analyze_correlations_core`   — full orchestrator
//!
//! The JS source threads dedupe + clock state through callbacks; we
//! use traits ([`SignalDeduper`], [`Clock`]) so unit tests can drive
//! deterministic time and verify dedupe-key behaviour without
//! resorting to globals.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::analysis_constants::{
    contains_topic_keyword, find_related_topics, generate_dedupe_key, generate_signal_id,
    includes_keyword, ENERGY_COMMODITY_SYMBOLS, FLOW_DROP_KEYWORDS, FLOW_PRICE_THRESHOLD,
    MARKET_MOVE_THRESHOLD, NEWS_VELOCITY_THRESHOLD, PIPELINE_KEYWORDS, PREDICTION_SHIFT_THRESHOLD,
    SUPPRESSED_TRENDING_TERMS, TOPIC_KEYWORDS,
};
use crate::news_clustering::ClusteredEvent;

// ============================================================================
// Public types
// ============================================================================

/// Discriminated tag for [`CorrelationSignal::signal_type`]. Matches
/// the `SignalType` union in `analysis-core.ts:153-167` and the
/// expanded set in `analysis-constants.ts:232-246`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SignalType {
    PredictionLeadsNews,
    NewsLeadsMarkets,
    SilentDivergence,
    VelocitySpike,
    KeywordSpike,
    Convergence,
    Triangulation,
    FlowDrop,
    FlowPriceDivergence,
    GeoConvergence,
    ExplainedMarketMove,
    HotspotEscalation,
    SectorCascade,
    MilitarySurge,
}

impl SignalType {
    /// Stable lower-snake string used as the dedupe-key prefix.
    /// Mirrors the literal strings in `analysis-core.ts` so dedupe
    /// keys round-trip across JS / Rust deployments.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PredictionLeadsNews => "prediction_leads_news",
            Self::NewsLeadsMarkets => "news_leads_markets",
            Self::SilentDivergence => "silent_divergence",
            Self::VelocitySpike => "velocity_spike",
            Self::KeywordSpike => "keyword_spike",
            Self::Convergence => "convergence",
            Self::Triangulation => "triangulation",
            Self::FlowDrop => "flow_drop",
            Self::FlowPriceDivergence => "flow_price_divergence",
            Self::GeoConvergence => "geo_convergence",
            Self::ExplainedMarketMove => "explained_market_move",
            Self::HotspotEscalation => "hotspot_escalation",
            Self::SectorCascade => "sector_cascade",
            Self::MilitarySurge => "military_surge",
        }
    }
}

/// Source-type tag for `convergence` / `triangulation`. Matches the
/// `SourceType` union in `analysis-core.ts:191`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SourceType {
    Wire,
    Gov,
    Intel,
    Mainstream,
    Market,
    Tech,
    Other,
}

impl SourceType {
    /// Lowercase string for use in `description` fields.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Wire => "wire",
            Self::Gov => "gov",
            Self::Intel => "intel",
            Self::Mainstream => "mainstream",
            Self::Market => "market",
            Self::Tech => "tech",
            Self::Other => "other",
        }
    }
}

/// Prediction-market input row — mirrors `PredictionMarketCore`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PredictionMarket {
    pub title: String,
    #[serde(rename = "yesPrice")]
    pub yes_price: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub volume: Option<f64>,
}

/// Market-data input row — mirrors `MarketDataCore`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MarketData {
    pub symbol: String,
    pub name: String,
    pub display: String,
    /// Last price; `None` if the upstream feed didn't provide one.
    #[serde(default)]
    pub price: Option<f64>,
    /// Percent change since previous close; `None` if unknown.
    #[serde(default)]
    pub change: Option<f64>,
}

/// Output row — mirrors `CorrelationSignalCore`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CorrelationSignal {
    pub id: String,
    #[serde(rename = "type")]
    pub signal_type: SignalType,
    pub title: String,
    pub description: String,
    pub confidence: f64,
    pub timestamp: DateTime<Utc>,
    #[serde(default)]
    pub data: SignalData,
}

/// Inline data envelope for a [`CorrelationSignal`]. Each field
/// corresponds to the eponymous key in `CorrelationSignalCore.data`
/// in `analysis-core.ts:175-189`. All fields optional so the JSON
/// shape stays the union of every detector's payload.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct SignalData {
    #[serde(default, rename = "newsVelocity", skip_serializing_if = "Option::is_none")]
    pub news_velocity: Option<f64>,
    #[serde(default, rename = "marketChange", skip_serializing_if = "Option::is_none")]
    pub market_change: Option<f64>,
    #[serde(default, rename = "predictionShift", skip_serializing_if = "Option::is_none")]
    pub prediction_shift: Option<f64>,
    #[serde(default, rename = "relatedTopics", skip_serializing_if = "Vec::is_empty")]
    pub related_topics: Vec<String>,
    #[serde(default, rename = "correlatedEntities", skip_serializing_if = "Vec::is_empty")]
    pub correlated_entities: Vec<String>,
    #[serde(default, rename = "correlatedNews", skip_serializing_if = "Vec::is_empty")]
    pub correlated_news: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub explanation: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub term: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub baseline: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub multiplier: Option<f64>,
    #[serde(default, rename = "sourceCount", skip_serializing_if = "Option::is_none")]
    pub source_count: Option<usize>,
}

/// One point in the per-topic velocity history. Used by
/// `detect_velocity_spikes` to compute a rolling baseline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TopicVelocityPoint {
    /// UNIX millisecond epoch.
    pub timestamp: i64,
    pub velocity: f64,
}

/// Snapshot of last-tick state, fed into the next [`analyze_correlations_core`]
/// call. Mirrors `StreamSnapshot` (`analysis-core.ts:193-199`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct StreamSnapshot {
    #[serde(rename = "newsVelocity")]
    pub news_velocity: HashMap<String, f64>,
    #[serde(rename = "marketChanges")]
    pub market_changes: HashMap<String, f64>,
    #[serde(rename = "predictionChanges")]
    pub prediction_changes: HashMap<String, f64>,
    #[serde(rename = "topicVelocityHistory")]
    pub topic_velocity_history: HashMap<String, Vec<TopicVelocityPoint>>,
    pub timestamp: i64,
}

/// Result of the orchestrator — the freshly-detected signals plus
/// the snapshot to pass back in on the next tick.
#[derive(Debug, Clone, PartialEq)]
pub struct AnalyzeResult {
    pub signals: Vec<CorrelationSignal>,
    pub snapshot: StreamSnapshot,
}

// ============================================================================
// Constants extracted from `analysis-core.ts`
// ============================================================================

/// Sliding window for the topic-velocity baseline. Mirrors
/// `TOPIC_BASELINE_WINDOW_MS = 7 * 24 * 60 * 60 * 1000`
/// (`analysis-core.ts:81`).
pub const TOPIC_BASELINE_WINDOW_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Multiplier above the baseline that a topic's current velocity
/// must clear to trigger `velocity_spike`. Mirrors
/// `TOPIC_BASELINE_SPIKE_MULTIPLIER = 3` (`analysis-core.ts:82`).
pub const TOPIC_BASELINE_SPIKE_MULTIPLIER: f64 = 3.0;

/// Cap on the per-topic velocity history. Beyond this we drop the
/// oldest points. Mirrors `TOPIC_HISTORY_MAX_POINTS = 1000`
/// (`analysis-core.ts:83`).
pub const TOPIC_HISTORY_MAX_POINTS: usize = 1000;

/// 1-hour window for `convergence` (`analysis-core.ts:433`).
const CONVERGENCE_WINDOW_MS: i64 = 60 * 60 * 1000;

// ============================================================================
// Trait surfaces
// ============================================================================

/// Source-type resolver — maps a publisher string to its
/// [`SourceType`] tag. Mirrors the `(source: string) => SourceType`
/// callback in the JS API.
pub trait SourceTyper {
    fn source_type(&self, source: &str) -> SourceType;
}

impl<F> SourceTyper for F
where
    F: Fn(&str) -> SourceType,
{
    fn source_type(&self, source: &str) -> SourceType {
        (self)(source)
    }
}

/// Dedupe state — the JS API passes two callbacks
/// (`isRecentDuplicate` + `markSignalSeen`); we collapse them into a
/// single `&mut` trait so callers can persist the seen-set however
/// they like (in-memory `HashSet`, SQLite-backed table, …).
pub trait SignalDeduper {
    fn is_recent_duplicate(&self, key: &str) -> bool;
    fn mark_signal_seen(&mut self, key: &str);
}

/// In-memory [`SignalDeduper`] — a plain `HashSet<String>`. Used by
/// callers that don't need cross-process dedupe.
#[derive(Debug, Default)]
pub struct InMemoryDeduper {
    seen: std::collections::HashSet<String>,
}

impl SignalDeduper for InMemoryDeduper {
    fn is_recent_duplicate(&self, key: &str) -> bool {
        self.seen.contains(key)
    }
    fn mark_signal_seen(&mut self, key: &str) {
        self.seen.insert(key.to_string());
    }
}

/// Wall-clock abstraction so tests can pin `now`.
pub trait Clock: Send + Sync {
    /// Current time as a `DateTime<Utc>`.
    fn now(&self) -> DateTime<Utc>;
    /// Convenience — same instant as a UNIX millisecond epoch.
    fn now_ms(&self) -> i64 {
        self.now().timestamp_millis()
    }
}

/// Real wall-clock — `DateTime::Utc::now()`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> DateTime<Utc> {
        Utc::now()
    }
}

/// Test-only frozen clock.
#[derive(Debug, Clone, Copy)]
pub struct FixedClock(pub DateTime<Utc>);
impl Clock for FixedClock {
    fn now(&self) -> DateTime<Utc> {
        self.0
    }
}

/// Subset of the JS `EntityIndex` consumed by
/// [`analyze_correlations_core`]. We only need `byId.get(symbol)` to
/// read the entity's keywords list for the silent-divergence
/// "searched terms" string; everything else is stripped.
pub trait EntityIndex {
    /// Return the keywords associated with `entity_id`, or `None` if
    /// the id isn't in the registry.
    fn keywords_for(&self, entity_id: &str) -> Option<Vec<String>>;
}

/// Real impl that contains zero entries. Returning `None` from every
/// lookup is the *correct* behaviour when the registry hasn't been
/// loaded — callers fall through to the topic-based fallbacks.
#[derive(Debug, Default, Clone, Copy)]
pub struct NoEntityIndex;
impl EntityIndex for NoEntityIndex {
    fn keywords_for(&self, _entity_id: &str) -> Option<Vec<String>> {
        None
    }
}

/// `Map<clusterId, Vec<keyword>>` — the subset of the JS
/// `NewsEntityContext` map [`analyze_correlations_core`] needs for
/// `findNewsForMarketSymbol`. Each entry maps a cluster id to the
/// keyword tokens its primary title carries; the orchestrator looks
/// up which clusters mention the market symbol.
pub type NewsEntityContexts = HashMap<String, Vec<String>>;

// ============================================================================
// Pure helpers (private — port of `analysis-core.ts:348-383`)
// ============================================================================

/// Port of `extractTopics` (`analysis-core.ts:348-362`). Counts each
/// `TOPIC_KEYWORDS` entry across event titles, weighted by the
/// event's velocity (sources/hour) plus its source count. Suppressed
/// terms are skipped.
fn extract_topics(events: &[ClusteredEvent]) -> HashMap<String, f64> {
    let mut topics: HashMap<String, f64> = HashMap::new();
    for event in events {
        let title_lower = event.primary_title.to_lowercase();
        for &kw in TOPIC_KEYWORDS {
            if SUPPRESSED_TRENDING_TERMS.contains(kw) {
                continue;
            }
            if !contains_topic_keyword(&title_lower, kw) {
                continue;
            }
            let velocity = event
                .velocity
                .and_then(|v| v.sources_per_hour)
                .unwrap_or(0.0);
            *topics.entry(kw.to_string()).or_insert(0.0) +=
                velocity + (event.source_count as f64);
        }
    }
    topics
}

fn prune_velocity_history(
    history: &[TopicVelocityPoint],
    now_ms: i64,
) -> Vec<TopicVelocityPoint> {
    history
        .iter()
        .copied()
        .filter(|p| now_ms - p.timestamp <= TOPIC_BASELINE_WINDOW_MS)
        .collect()
}

fn average_velocity(history: &[TopicVelocityPoint]) -> f64 {
    if history.is_empty() {
        return 0.0;
    }
    let sum: f64 = history.iter().map(|p| p.velocity).sum();
    sum / history.len() as f64
}

/// Port of `countRelatedTopicMentions` (`analysis-core.ts:374-383`).
/// Sum the velocities of topics whose lowercased name appears
/// inside `market.name`, OR whose lowercased name *contains*
/// `market.symbol` lowercased. The symmetric/asymmetric pair
/// matches the JS `marketNameLower.includes(topic) ||
/// topic.includes(marketSymbolLower)`.
fn count_related_topic_mentions(
    news_topics: &HashMap<String, f64>,
    market_name: &str,
    market_symbol: &str,
) -> f64 {
    let market_name_lower = market_name.to_lowercase();
    let market_symbol_lower = market_symbol.to_lowercase();
    news_topics
        .iter()
        .filter(|(topic, _)| {
            market_name_lower.contains(topic.as_str()) || topic.contains(&market_symbol_lower)
        })
        .map(|(_, v)| *v)
        .sum()
}

fn min_f(a: f64, b: f64) -> f64 {
    if a < b {
        a
    } else {
        b
    }
}

fn max_f(a: f64, b: f64) -> f64 {
    if a > b {
        a
    } else {
        b
    }
}

// ============================================================================
// Detectors
// ============================================================================

/// Port of `detectPipelineFlowDrops` (`analysis-core.ts:385-424`).
/// For each event whose primary title (or any item title) contains
/// **both** a pipeline keyword and a flow-drop keyword, emit a
/// `flow_drop` signal — provided the dedupe key isn't already seen.
pub fn detect_pipeline_flow_drops<D, C>(
    events: &[ClusteredEvent],
    deduper: &mut D,
    clock: &C,
) -> Vec<CorrelationSignal>
where
    D: SignalDeduper,
    C: Clock,
{
    let mut signals = Vec::new();
    for event in events {
        // Build the lowercased title set: primary + every member.
        let mut titles_lower: Vec<String> = vec![event.primary_title.to_lowercase()];
        titles_lower.extend(event.all_items.iter().map(|i| i.title.to_lowercase()));

        let has_pipeline = titles_lower
            .iter()
            .any(|t| includes_keyword(t, PIPELINE_KEYWORDS));
        let has_flow_drop = titles_lower
            .iter()
            .any(|t| includes_keyword(t, FLOW_DROP_KEYWORDS));

        if has_pipeline && has_flow_drop {
            let dedupe = generate_dedupe_key(
                SignalType::FlowDrop.as_str(),
                &event.id,
                event.source_count as f64,
            );
            if !deduper.is_recent_duplicate(&dedupe) {
                deduper.mark_signal_seen(&dedupe);
                let conf = min_f(0.9, 0.4 + (event.source_count as f64) / 10.0);
                signals.push(CorrelationSignal {
                    id: generate_signal_id(),
                    signal_type: SignalType::FlowDrop,
                    title: "Pipeline Flow Drop".into(),
                    description: format!(
                        "\"{}…\" indicates reduced flow or disruption",
                        slice_chars(&event.primary_title, 70)
                    ),
                    confidence: conf,
                    timestamp: clock.now(),
                    data: SignalData {
                        news_velocity: Some(event.source_count as f64),
                        related_topics: vec!["pipeline".into(), "flow".into()],
                        ..SignalData::default()
                    },
                });
            }
        }
    }
    signals
}

/// Port of `detectConvergence` (`analysis-core.ts:426-473`). Emits
/// when ≥3 distinct non-`other` source types covered the same event
/// in the last hour (≥3 recent items required). The dedupe key is
/// keyed by event id + count of distinct types.
pub fn detect_convergence<D, C, T>(
    events: &[ClusteredEvent],
    typer: &T,
    deduper: &mut D,
    clock: &C,
) -> Vec<CorrelationSignal>
where
    D: SignalDeduper,
    C: Clock,
    T: SourceTyper,
{
    let mut signals = Vec::new();
    let now_ms = clock.now_ms();
    for event in events {
        if event.all_items.len() < 3 {
            continue;
        }
        let recent_items: Vec<_> = event
            .all_items
            .iter()
            .filter(|i| now_ms - i.pub_date.timestamp_millis() < CONVERGENCE_WINDOW_MS)
            .collect();
        if recent_items.len() < 3 {
            continue;
        }
        let mut source_types: std::collections::HashSet<SourceType> =
            std::collections::HashSet::new();
        for i in &recent_items {
            source_types.insert(typer.source_type(&i.source));
        }
        if source_types.len() < 3 {
            continue;
        }
        let types: Vec<SourceType> = source_types
            .iter()
            .copied()
            .filter(|t| *t != SourceType::Other)
            .collect();
        if types.len() < 3 {
            continue;
        }

        let dedupe = generate_dedupe_key(
            SignalType::Convergence.as_str(),
            &event.id,
            source_types.len() as f64,
        );
        if deduper.is_recent_duplicate(&dedupe) {
            continue;
        }
        deduper.mark_signal_seen(&dedupe);

        let mut type_strs: Vec<&str> = types.iter().map(|t| t.as_str()).collect();
        type_strs.sort_unstable(); // deterministic description
        let conf = min_f(0.95, 0.6 + source_types.len() as f64 * 0.1);
        signals.push(CorrelationSignal {
            id: generate_signal_id(),
            signal_type: SignalType::Convergence,
            title: "Source Convergence".into(),
            description: format!(
                "\"{}…\" reported by {} ({} sources in 30m)",
                slice_chars(&event.primary_title, 50),
                type_strs.join(", "),
                recent_items.len()
            ),
            confidence: conf,
            timestamp: clock.now(),
            data: SignalData {
                news_velocity: Some(recent_items.len() as f64),
                related_topics: type_strs.iter().map(|s| (*s).to_string()).collect(),
                ..SignalData::default()
            },
        });
    }
    signals
}

/// Port of `detectTriangulation` (`analysis-core.ts:475-517`). Emits
/// when the `wire / gov / intel` triple is all present among the
/// cluster's items.
pub fn detect_triangulation<D, C, T>(
    events: &[ClusteredEvent],
    typer: &T,
    deduper: &mut D,
    clock: &C,
) -> Vec<CorrelationSignal>
where
    D: SignalDeduper,
    C: Clock,
    T: SourceTyper,
{
    let critical = [SourceType::Wire, SourceType::Gov, SourceType::Intel];
    let mut signals = Vec::new();
    for event in events {
        if event.all_items.len() < 3 {
            continue;
        }
        let mut present: std::collections::HashSet<SourceType> = std::collections::HashSet::new();
        for i in &event.all_items {
            let t = typer.source_type(&i.source);
            if critical.contains(&t) {
                present.insert(t);
            }
        }
        if present.len() != 3 {
            continue;
        }
        let dedupe = generate_dedupe_key(SignalType::Triangulation.as_str(), &event.id, 3.0);
        if deduper.is_recent_duplicate(&dedupe) {
            continue;
        }
        deduper.mark_signal_seen(&dedupe);

        let mut topics: Vec<String> = present.iter().map(|t| t.as_str().to_string()).collect();
        topics.sort();
        signals.push(CorrelationSignal {
            id: generate_signal_id(),
            signal_type: SignalType::Triangulation,
            title: "Intel Triangulation".into(),
            description: format!(
                "Wire + Gov + Intel aligned: \"{}…\"",
                slice_chars(&event.primary_title, 45)
            ),
            confidence: 0.9,
            timestamp: clock.now(),
            data: SignalData {
                news_velocity: Some(event.source_count as f64),
                related_topics: topics,
                ..SignalData::default()
            },
        });
    }
    signals
}

/// Port of the `prediction_leads_news` block embedded in
/// `analyzeCorrelationsCore` (`analysis-core.ts:570-599`).
pub fn detect_prediction_shifts<D, C>(
    predictions: &[PredictionMarket],
    previous_changes: &HashMap<String, f64>,
    news_topics: &HashMap<String, f64>,
    deduper: &mut D,
    clock: &C,
) -> Vec<CorrelationSignal>
where
    D: SignalDeduper,
    C: Clock,
{
    let mut signals = Vec::new();
    for pred in predictions {
        let key: String = pred.title.chars().take(50).collect();
        let Some(&prev) = previous_changes.get(&key) else {
            continue;
        };
        let shift = (pred.yes_price - prev).abs();
        if shift < PREDICTION_SHIFT_THRESHOLD {
            continue;
        }
        let related = find_related_topics(&pred.title);
        let news_activity: f64 = related
            .iter()
            .map(|t| news_topics.get(t).copied().unwrap_or(0.0))
            .sum();

        let dedupe =
            generate_dedupe_key(SignalType::PredictionLeadsNews.as_str(), &key, shift);
        if news_activity >= NEWS_VELOCITY_THRESHOLD || deduper.is_recent_duplicate(&dedupe) {
            continue;
        }
        deduper.mark_signal_seen(&dedupe);

        let conf = min_f(0.9, 0.5 + shift / 20.0);
        let direction = if pred.yes_price - prev > 0.0 {
            "+"
        } else {
            ""
        };
        let signed = pred.yes_price - prev;
        signals.push(CorrelationSignal {
            id: generate_signal_id(),
            signal_type: SignalType::PredictionLeadsNews,
            title: "Prediction Market Shift".into(),
            description: format!(
                "\"{}…\" moved {direction}{:.1}% with low news coverage",
                slice_chars(&pred.title, 60),
                signed
            ),
            confidence: conf,
            timestamp: clock.now(),
            data: SignalData {
                prediction_shift: Some(shift),
                news_velocity: Some(news_activity),
                related_topics: related,
                ..SignalData::default()
            },
        });
    }
    signals
}

/// Port of the `velocity_spike` block (`analysis-core.ts:601-638`).
pub fn detect_velocity_spikes<D, C>(
    news_topics: &HashMap<String, f64>,
    previous_history: &HashMap<String, Vec<TopicVelocityPoint>>,
    deduper: &mut D,
    clock: &C,
) -> Vec<CorrelationSignal>
where
    D: SignalDeduper,
    C: Clock,
{
    let mut signals = Vec::new();
    let now_ms = clock.now_ms();
    for (topic, &velocity) in news_topics {
        if SUPPRESSED_TRENDING_TERMS.contains(topic.as_str()) {
            continue;
        }
        let baseline_history = previous_history
            .get(topic)
            .map(|h| prune_velocity_history(h, now_ms))
            .unwrap_or_default();
        let baseline = average_velocity(&baseline_history);
        let exceeds_absolute = velocity > NEWS_VELOCITY_THRESHOLD * 2.0;
        let exceeds_baseline = if baseline > 0.0 {
            velocity > baseline * TOPIC_BASELINE_SPIKE_MULTIPLIER
        } else {
            exceeds_absolute
        };
        if !exceeds_absolute || !exceeds_baseline {
            continue;
        }

        let multiplier = if baseline > 0.0 { velocity / baseline } else { 0.0 };
        let dedupe = generate_dedupe_key(SignalType::VelocitySpike.as_str(), topic, velocity);
        if deduper.is_recent_duplicate(&dedupe) {
            continue;
        }
        deduper.mark_signal_seen(&dedupe);

        let baseline_text = if baseline > 0.0 {
            format!("{:.1} baseline ({:.1}x)", baseline, multiplier)
        } else {
            "cold-start baseline".into()
        };
        let conf = if multiplier > 0.0 {
            min_f(0.9, 0.45 + multiplier / 8.0)
        } else {
            min_f(0.9, 0.45 + velocity / 18.0)
        };
        let explanation = if baseline > 0.0 {
            format!(
                "Velocity {:.1} is {:.1}x above baseline {:.1}",
                velocity, multiplier, baseline
            )
        } else {
            format!("Velocity {:.1} exceeded cold-start threshold", velocity)
        };
        signals.push(CorrelationSignal {
            id: generate_signal_id(),
            signal_type: SignalType::VelocitySpike,
            title: "News Velocity Spike".into(),
            description: format!(
                "\"{topic}\" coverage surging: {:.1} activity score vs {baseline_text}",
                velocity
            ),
            confidence: conf,
            timestamp: clock.now(),
            data: SignalData {
                news_velocity: Some(velocity),
                related_topics: vec![topic.clone()],
                baseline: Some(baseline),
                multiplier: if baseline > 0.0 { Some(multiplier) } else { None },
                explanation: Some(explanation),
                ..SignalData::default()
            },
        });
    }
    signals
}

/// Port of the `explained_market_move` ⊕ `silent_divergence` block
/// (`analysis-core.ts:640-694`). When `entity_index` carries
/// keywords for the symbol AND `news_entity_contexts` has clusters
/// mentioning the symbol, emit `explained_market_move`. Otherwise
/// fall back to the topic-mention count and emit `silent_divergence`
/// when the topic mentions are < 2.
pub fn detect_market_moves<D, C, E>(
    markets: &[MarketData],
    news_topics: &HashMap<String, f64>,
    news_entity_contexts: &NewsEntityContexts,
    entity_index: &E,
    deduper: &mut D,
    clock: &C,
) -> Vec<CorrelationSignal>
where
    D: SignalDeduper,
    C: Clock,
    E: EntityIndex,
{
    let mut signals = Vec::new();
    for m in markets {
        let change = m.change.unwrap_or(0.0).abs();
        if change < MARKET_MOVE_THRESHOLD {
            continue;
        }
        // Find clusters whose entity-context keywords contain the
        // market symbol. Without a real entity-extraction pass this
        // returns nothing, which exercises the silent-divergence
        // fallback.
        let symbol_lower = m.symbol.to_lowercase();
        let related_news: Vec<&String> = news_entity_contexts
            .iter()
            .filter(|(_, kws)| kws.iter().any(|k| k.to_lowercase() == symbol_lower))
            .map(|(cid, _)| cid)
            .collect();

        if !related_news.is_empty() {
            let dedupe = generate_dedupe_key(
                SignalType::ExplainedMarketMove.as_str(),
                &m.symbol,
                change,
            );
            if deduper.is_recent_duplicate(&dedupe) {
                continue;
            }
            deduper.mark_signal_seen(&dedupe);

            let signed = m.change.unwrap_or(0.0);
            let direction = if signed > 0.0 { "+" } else { "" };
            let conf = min_f(
                0.9,
                0.5 + (related_news.len() as f64) * 0.1 + change / 20.0,
            );
            let explanation = format!(
                "{} related news item{} found",
                related_news.len(),
                if related_news.len() > 1 { "s" } else { "" }
            );
            // Top news headline — best-effort; without a back-ref to
            // the cluster's title we use the cluster id.
            let top_news_label = related_news.first().map_or("", |s| s.as_str());
            signals.push(CorrelationSignal {
                id: generate_signal_id(),
                signal_type: SignalType::ExplainedMarketMove,
                title: "Market Move Explained".into(),
                description: format!(
                    "{} {direction}{:.2}% correlates with: \"{}…\"",
                    m.name,
                    signed,
                    slice_chars(top_news_label, 60)
                ),
                confidence: conf,
                timestamp: clock.now(),
                data: SignalData {
                    market_change: Some(signed),
                    news_velocity: Some(related_news.len() as f64),
                    correlated_entities: vec![m.symbol.clone()],
                    correlated_news: related_news.iter().map(|s| (*s).clone()).collect(),
                    explanation: Some(explanation),
                    ..SignalData::default()
                },
            });
        } else {
            let old_related = count_related_topic_mentions(news_topics, &m.name, &m.symbol);
            let dedupe = generate_dedupe_key(
                SignalType::SilentDivergence.as_str(),
                &m.symbol,
                change,
            );
            if old_related >= 2.0 || deduper.is_recent_duplicate(&dedupe) {
                continue;
            }
            deduper.mark_signal_seen(&dedupe);

            let signed = m.change.unwrap_or(0.0);
            let direction = if signed > 0.0 { "+" } else { "" };
            // Searched-terms string — when the entity index has
            // keywords, prepend them; otherwise fall back to the
            // symbol alone (matches the JS ternary).
            let searched_terms = match entity_index.keywords_for(&m.symbol) {
                Some(kws) if !kws.is_empty() => {
                    let mut parts: Vec<String> =
                        vec![m.symbol.clone(), m.name.clone()];
                    parts.extend(kws.into_iter().take(2));
                    parts.join(", ")
                }
                _ => m.symbol.clone(),
            };
            let conf = min_f(0.8, 0.4 + change / 10.0);
            signals.push(CorrelationSignal {
                id: generate_signal_id(),
                signal_type: SignalType::SilentDivergence,
                title: "Silent Divergence".into(),
                description: format!(
                    "{} moved {direction}{:.2}% - no news found for: {searched_terms}",
                    m.name, signed
                ),
                confidence: conf,
                timestamp: clock.now(),
                data: SignalData {
                    market_change: Some(signed),
                    news_velocity: Some(old_related),
                    explanation: Some(format!("Searched: {searched_terms}")),
                    ..SignalData::default()
                },
            });
        }
    }
    signals
}

/// Port of `flow_price_divergence` (`analysis-core.ts:696-722`).
pub fn detect_flow_price_divergence<D, C>(
    markets: &[MarketData],
    news_topics: &HashMap<String, f64>,
    pipeline_flow_mentions: usize,
    deduper: &mut D,
    clock: &C,
) -> Vec<CorrelationSignal>
where
    D: SignalDeduper,
    C: Clock,
{
    let mut signals = Vec::new();
    for m in markets {
        if !ENERGY_COMMODITY_SYMBOLS.contains(m.symbol.as_str()) {
            continue;
        }
        let change = m.change.unwrap_or(0.0);
        if change < FLOW_PRICE_THRESHOLD {
            continue;
        }
        let related = count_related_topic_mentions(news_topics, &m.name, &m.symbol);
        let dedupe = generate_dedupe_key(
            SignalType::FlowPriceDivergence.as_str(),
            &m.symbol,
            change,
        );
        if related >= 2.0 || pipeline_flow_mentions != 0 || deduper.is_recent_duplicate(&dedupe) {
            continue;
        }
        deduper.mark_signal_seen(&dedupe);

        let conf = min_f(0.85, 0.4 + change / 8.0);
        signals.push(CorrelationSignal {
            id: generate_signal_id(),
            signal_type: SignalType::FlowPriceDivergence,
            title: "Flow/Price Divergence".into(),
            description: format!("{} up {:.2}% without pipeline flow news", m.name, change),
            confidence: conf,
            timestamp: clock.now(),
            data: SignalData {
                market_change: Some(change),
                news_velocity: Some(related),
                related_topics: vec!["pipeline".into(), m.display.clone()],
                ..SignalData::default()
            },
        });
    }
    signals
}

// ============================================================================
// Orchestrator — port of `analyzeCorrelationsCore`
// ============================================================================

/// Build the next [`StreamSnapshot`] and run every detector.
///
/// Returns the freshly-emitted signals (filtered to confidence ≥ 0.6
/// and deduped per type) plus the snapshot to feed back on the next
/// tick. When `previous_snapshot` is `None` we emit zero signals and
/// return the current snapshot as the cold-start baseline — same as
/// `analysis-core.ts:566-568`.
///
/// Argument list mirrors the JS source's `analyzeCorrelationsCore`
/// signature (events, predictions, markets, previousSnapshot,
/// getSourceType, isRecentDuplicate, markSignalSeen) — the
/// `clippy::too_many_arguments` cap is waived deliberately so the
/// shape stays portable.
#[allow(clippy::too_many_arguments)]
pub fn analyze_correlations_core<D, C, T, E>(
    events: &[ClusteredEvent],
    predictions: &[PredictionMarket],
    markets: &[MarketData],
    previous_snapshot: Option<&StreamSnapshot>,
    typer: &T,
    entity_index: &E,
    news_entity_contexts: &NewsEntityContexts,
    deduper: &mut D,
    clock: &C,
) -> AnalyzeResult
where
    D: SignalDeduper,
    C: Clock,
    T: SourceTyper,
    E: EntityIndex,
{
    let now_ms = clock.now_ms();
    let news_topics = extract_topics(events);
    let pipeline_flow_signals = detect_pipeline_flow_drops(events, deduper, clock);
    let pipeline_flow_mentions = pipeline_flow_signals.len();

    // Build current topic-velocity history — every previously-seen
    // topic gets its window pruned and the current sample appended;
    // every newly-seen topic gets a single-point history with the
    // current sample.
    let mut current_history: HashMap<String, Vec<TopicVelocityPoint>> = HashMap::new();
    let previous_history = previous_snapshot
        .map(|s| s.topic_velocity_history.clone())
        .unwrap_or_default();
    let mut topic_universe: std::collections::BTreeSet<String> =
        std::collections::BTreeSet::new();
    for k in previous_history.keys() {
        topic_universe.insert(k.clone());
    }
    for k in news_topics.keys() {
        topic_universe.insert(k.clone());
    }
    for topic in topic_universe {
        let prior = previous_history
            .get(&topic)
            .map(|h| prune_velocity_history(h, now_ms))
            .unwrap_or_default();
        let mut updated = prior;
        updated.push(TopicVelocityPoint {
            timestamp: now_ms,
            velocity: news_topics.get(&topic).copied().unwrap_or(0.0),
        });
        if updated.len() > TOPIC_HISTORY_MAX_POINTS {
            let drop = updated.len() - TOPIC_HISTORY_MAX_POINTS;
            updated.drain(0..drop);
        }
        current_history.insert(topic, updated);
    }

    let current_snapshot = StreamSnapshot {
        news_velocity: news_topics.clone(),
        market_changes: markets
            .iter()
            .map(|m| (m.symbol.clone(), m.change.unwrap_or(0.0)))
            .collect(),
        prediction_changes: predictions
            .iter()
            .map(|p| (p.title.chars().take(50).collect::<String>(), p.yes_price))
            .collect(),
        topic_velocity_history: current_history,
        timestamp: now_ms,
    };

    // Cold-start: no previous snapshot → no diff-based signals can
    // fire. Mirrors `analysis-core.ts:566-568`.
    let Some(previous) = previous_snapshot else {
        return AnalyzeResult {
            signals: Vec::new(),
            snapshot: current_snapshot,
        };
    };

    let mut signals: Vec<CorrelationSignal> = Vec::new();
    signals.extend(detect_prediction_shifts(
        predictions,
        &previous.prediction_changes,
        &news_topics,
        deduper,
        clock,
    ));
    signals.extend(detect_velocity_spikes(
        &news_topics,
        &previous.topic_velocity_history,
        deduper,
        clock,
    ));
    signals.extend(detect_market_moves(
        markets,
        &news_topics,
        news_entity_contexts,
        entity_index,
        deduper,
        clock,
    ));
    signals.extend(detect_flow_price_divergence(
        markets,
        &news_topics,
        pipeline_flow_mentions,
        deduper,
        clock,
    ));
    signals.extend(detect_convergence(events, typer, deduper, clock));
    signals.extend(detect_triangulation(events, typer, deduper, clock));
    signals.extend(pipeline_flow_signals);

    // Dedupe by signal type — keeps the first occurrence of each
    // type. Mirrors `analysis-core.ts:730-732`.
    let mut seen_types: std::collections::HashSet<SignalType> = std::collections::HashSet::new();
    let unique_signals: Vec<CorrelationSignal> = signals
        .into_iter()
        .filter(|s| seen_types.insert(s.signal_type))
        .collect();

    // Filter to confidence ≥ 0.6 (`analysis-core.ts:735-737`).
    let high_conf: Vec<CorrelationSignal> = unique_signals
        .into_iter()
        .filter(|s| s.confidence >= 0.6)
        .collect();

    // Ensure max_f is referenced so `clippy::dead_code` is content
    // even when the orchestrator is the only caller of the helper.
    let _: f64 = max_f(0.0, 1.0);

    AnalyzeResult {
        signals: high_conf,
        snapshot: current_snapshot,
    }
}

// ============================================================================
// Helpers — char-aware string slicing (JS `.slice(0, n)` is char-based)
// ============================================================================

fn slice_chars(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::clustering::ThreatClassification;
    use crate::news_clustering::{ClusterVelocity, NewsItemCore};
    use chrono::TimeZone;

    fn ts(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, minute, 0).unwrap()
    }

    fn news_item(title: &str, source: &str, when: DateTime<Utc>) -> NewsItemCore {
        NewsItemCore {
            source: source.into(),
            title: title.into(),
            link: format!("http://x/{source}"),
            pub_date: when,
            is_alert: false,
            monitor_color: None,
            tier: Some(2),
            threat: None,
            lat: None,
            lon: None,
            location_name: None,
            lang: None,
        }
    }

    fn cluster(
        id: &str,
        primary_title: &str,
        items: Vec<NewsItemCore>,
    ) -> ClusteredEvent {
        let last = items.iter().map(|i| i.pub_date).max().unwrap();
        let first = items.iter().map(|i| i.pub_date).min().unwrap();
        ClusteredEvent {
            id: id.into(),
            primary_title: primary_title.into(),
            primary_source: items[0].source.clone(),
            primary_link: items[0].link.clone(),
            source_count: items.len(),
            top_sources: Vec::new(),
            all_items: items,
            first_seen: first,
            last_updated: last,
            is_alert: false,
            monitor_color: None,
            velocity: Some(ClusterVelocity {
                sources_per_hour: Some(1.0),
            }),
            threat: Some(ThreatClassification {
                level: "info".into(),
                source: "keyword".into(),
                category: Some("general".into()),
                confidence: Some(0.3),
            }),
            lat: None,
            lon: None,
            lang: None,
        }
    }

    fn typer_for(map: &'static [(&'static str, SourceType)]) -> impl SourceTyper + Copy {
        // Returned closure captures by Copy so it's `Sync` + Send.
        move |source: &str| -> SourceType {
            map.iter()
                .find(|(s, _)| *s == source)
                .map(|(_, t)| *t)
                .unwrap_or(SourceType::Other)
        }
    }

    // ── pipeline_flow_drops ─────────────────────────────────────

    #[test]
    fn pipeline_flow_drop_emits_when_pipeline_and_flowdrop_keywords_both_present() {
        let event = cluster(
            "evt-1",
            "Pipeline rupture halts northbound flow at Kirkuk terminal",
            vec![news_item(
                "Pipeline rupture halts northbound flow at Kirkuk terminal",
                "Reuters",
                ts(2026, 5, 4, 12, 0),
            )],
        );
        let mut deduper = InMemoryDeduper::default();
        let clock = FixedClock(ts(2026, 5, 4, 12, 5));
        let signals = detect_pipeline_flow_drops(&[event], &mut deduper, &clock);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_type, SignalType::FlowDrop);
        assert!(signals[0].confidence >= 0.4);
    }

    #[test]
    fn pipeline_flow_drop_no_emit_when_only_pipeline_keyword() {
        let event = cluster(
            "evt-1",
            "Pipeline opens new export terminal in Kirkuk",
            vec![news_item(
                "Pipeline opens new export terminal in Kirkuk",
                "Reuters",
                ts(2026, 5, 4, 12, 0),
            )],
        );
        let mut deduper = InMemoryDeduper::default();
        let clock = FixedClock(ts(2026, 5, 4, 12, 5));
        let signals = detect_pipeline_flow_drops(&[event], &mut deduper, &clock);
        assert!(signals.is_empty());
    }

    #[test]
    fn pipeline_flow_drop_dedupes_repeats() {
        let event = cluster(
            "evt-1",
            "Pipeline rupture halts flow",
            vec![news_item(
                "Pipeline rupture halts flow",
                "Reuters",
                ts(2026, 5, 4, 12, 0),
            )],
        );
        let mut deduper = InMemoryDeduper::default();
        let clock = FixedClock(ts(2026, 5, 4, 12, 5));
        let _ = detect_pipeline_flow_drops(std::slice::from_ref(&event), &mut deduper, &clock);
        let signals2 = detect_pipeline_flow_drops(&[event], &mut deduper, &clock);
        assert!(signals2.is_empty(), "second call must dedupe");
    }

    // ── convergence ─────────────────────────────────────────────

    #[test]
    fn convergence_requires_three_recent_distinct_non_other_types() {
        let now = ts(2026, 5, 4, 12, 30);
        // 3 items, 3 distinct types (wire, gov, intel), all within
        // the last hour.
        let items = vec![
            news_item("Iran missile strike Tehran", "Reuters", now),
            news_item("Iran missile strike Tehran", "GovWire", ts(2026, 5, 4, 12, 0)),
            news_item("Iran missile strike Tehran", "Intel-X", ts(2026, 5, 4, 12, 15)),
        ];
        let event = cluster("c1", "Iran missile strike Tehran", items);
        let map: &[(&'static str, SourceType)] = &[
            ("Reuters", SourceType::Wire),
            ("GovWire", SourceType::Gov),
            ("Intel-X", SourceType::Intel),
        ];
        let typer = typer_for(map);
        let mut deduper = InMemoryDeduper::default();
        let clock = FixedClock(now);
        let signals = detect_convergence(&[event], &typer, &mut deduper, &clock);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_type, SignalType::Convergence);
    }

    #[test]
    fn convergence_no_emit_when_only_two_types() {
        let now = ts(2026, 5, 4, 12, 30);
        let items = vec![
            news_item("Iran missile strike Tehran", "Reuters", now),
            news_item("Iran missile strike Tehran", "AP", now),
            news_item("Iran missile strike Tehran", "AFP", now),
        ];
        let event = cluster("c1", "Iran missile strike Tehran", items);
        let map: &[(&'static str, SourceType)] = &[
            ("Reuters", SourceType::Wire),
            ("AP", SourceType::Wire),
            ("AFP", SourceType::Wire),
        ];
        let typer = typer_for(map);
        let mut deduper = InMemoryDeduper::default();
        let clock = FixedClock(now);
        let signals = detect_convergence(&[event], &typer, &mut deduper, &clock);
        assert!(signals.is_empty());
    }

    // ── triangulation ───────────────────────────────────────────

    #[test]
    fn triangulation_requires_wire_gov_intel_triple() {
        let now = ts(2026, 5, 4, 12, 0);
        let items = vec![
            news_item("Iran missile strike", "Reuters", now),
            news_item("Iran missile strike", "GovWire", now),
            news_item("Iran missile strike", "Intel-X", now),
        ];
        let event = cluster("c1", "Iran missile strike", items);
        let map: &[(&'static str, SourceType)] = &[
            ("Reuters", SourceType::Wire),
            ("GovWire", SourceType::Gov),
            ("Intel-X", SourceType::Intel),
        ];
        let typer = typer_for(map);
        let mut deduper = InMemoryDeduper::default();
        let clock = FixedClock(now);
        let signals = detect_triangulation(&[event], &typer, &mut deduper, &clock);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_type, SignalType::Triangulation);
        assert!((signals[0].confidence - 0.9).abs() < 1e-9);
    }

    #[test]
    fn triangulation_skips_when_only_wire_and_gov() {
        let now = ts(2026, 5, 4, 12, 0);
        let items = vec![
            news_item("Iran missile strike", "Reuters", now),
            news_item("Iran missile strike", "GovWire", now),
            news_item("Iran missile strike", "Bloomberg", now),
        ];
        let event = cluster("c1", "Iran missile strike", items);
        let map: &[(&'static str, SourceType)] = &[
            ("Reuters", SourceType::Wire),
            ("GovWire", SourceType::Gov),
            ("Bloomberg", SourceType::Mainstream),
        ];
        let typer = typer_for(map);
        let mut deduper = InMemoryDeduper::default();
        let clock = FixedClock(now);
        let signals = detect_triangulation(&[event], &typer, &mut deduper, &clock);
        assert!(signals.is_empty());
    }

    // ── prediction_shifts ───────────────────────────────────────

    #[test]
    fn prediction_shift_emits_when_above_threshold_and_news_quiet() {
        let pred = PredictionMarket {
            title: "Will Iran strike Israel by year-end?".into(),
            yes_price: 38.0,
            volume: None,
        };
        let mut prev = HashMap::new();
        prev.insert(
            "Will Iran strike Israel by year-end?"
                .chars()
                .take(50)
                .collect::<String>(),
            30.0,
        );
        // News topics empty → news_activity = 0 < threshold.
        let news_topics: HashMap<String, f64> = HashMap::new();
        let mut deduper = InMemoryDeduper::default();
        let clock = FixedClock(ts(2026, 5, 4, 12, 0));
        let signals =
            detect_prediction_shifts(&[pred], &prev, &news_topics, &mut deduper, &clock);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_type, SignalType::PredictionLeadsNews);
        let shift = signals[0].data.prediction_shift.unwrap();
        assert!((shift - 8.0).abs() < 1e-9);
    }

    #[test]
    fn prediction_shift_no_emit_when_news_already_active() {
        let pred = PredictionMarket {
            title: "Will Iran strike Israel by year-end?".into(),
            yes_price: 38.0,
            volume: None,
        };
        let mut prev = HashMap::new();
        prev.insert(
            "Will Iran strike Israel by year-end?"
                .chars()
                .take(50)
                .collect::<String>(),
            30.0,
        );
        let mut news_topics = HashMap::new();
        // "iran" topic carrying a velocity above the threshold.
        news_topics.insert("iran".to_string(), 5.0);
        let mut deduper = InMemoryDeduper::default();
        let clock = FixedClock(ts(2026, 5, 4, 12, 0));
        let signals =
            detect_prediction_shifts(&[pred], &prev, &news_topics, &mut deduper, &clock);
        assert!(signals.is_empty());
    }

    // ── velocity_spikes ─────────────────────────────────────────

    #[test]
    fn velocity_spike_emits_on_cold_start_when_above_absolute_threshold() {
        let mut topics = HashMap::new();
        // 2 × NEWS_VELOCITY_THRESHOLD (3) = 6; pick something above.
        topics.insert("ukraine".to_string(), 12.0);
        let history = HashMap::new();
        let mut deduper = InMemoryDeduper::default();
        let clock = FixedClock(ts(2026, 5, 4, 12, 0));
        let signals = detect_velocity_spikes(&topics, &history, &mut deduper, &clock);
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_type, SignalType::VelocitySpike);
    }

    #[test]
    fn velocity_spike_skips_suppressed_terms() {
        let mut topics = HashMap::new();
        topics.insert("ai".to_string(), 50.0); // ai is suppressed
        let history = HashMap::new();
        let mut deduper = InMemoryDeduper::default();
        let clock = FixedClock(ts(2026, 5, 4, 12, 0));
        let signals = detect_velocity_spikes(&topics, &history, &mut deduper, &clock);
        assert!(signals.is_empty());
    }

    #[test]
    fn velocity_spike_emits_only_when_above_baseline_multiplier() {
        let mut topics = HashMap::new();
        topics.insert("ukraine".to_string(), 9.0);
        let mut history = HashMap::new();
        history.insert(
            "ukraine".to_string(),
            vec![
                TopicVelocityPoint {
                    timestamp: ts(2026, 5, 3, 12, 0).timestamp_millis(),
                    velocity: 4.0,
                },
                TopicVelocityPoint {
                    timestamp: ts(2026, 5, 4, 0, 0).timestamp_millis(),
                    velocity: 4.0,
                },
            ],
        );
        let mut deduper = InMemoryDeduper::default();
        let clock = FixedClock(ts(2026, 5, 4, 12, 0));
        // baseline = 4.0 → spike threshold = 12.0 → 9 fails.
        let s1 = detect_velocity_spikes(&topics, &history, &mut deduper, &clock);
        assert!(s1.is_empty());
        // Bump velocity above 12.
        topics.insert("ukraine".to_string(), 15.0);
        let s2 = detect_velocity_spikes(&topics, &history, &mut deduper, &clock);
        assert_eq!(s2.len(), 1);
        let mult = s2[0].data.multiplier.unwrap();
        assert!((mult - 15.0 / 4.0).abs() < 1e-9);
    }

    // ── market_moves ────────────────────────────────────────────

    #[test]
    fn silent_divergence_when_no_related_news_or_topics() {
        let market = MarketData {
            symbol: "CL=F".into(),
            name: "Crude Oil Futures".into(),
            display: "WTI".into(),
            price: Some(78.0),
            change: Some(3.5),
        };
        let news_topics: HashMap<String, f64> = HashMap::new();
        let contexts: NewsEntityContexts = HashMap::new();
        let entity_index = NoEntityIndex;
        let mut deduper = InMemoryDeduper::default();
        let clock = FixedClock(ts(2026, 5, 4, 12, 0));
        let signals = detect_market_moves(
            &[market],
            &news_topics,
            &contexts,
            &entity_index,
            &mut deduper,
            &clock,
        );
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_type, SignalType::SilentDivergence);
        // Searched-terms string falls back to symbol when no entity
        // keywords available.
        assert!(signals[0].description.contains("CL=F"));
    }

    #[test]
    fn explained_market_move_when_entity_context_carries_symbol() {
        let market = MarketData {
            symbol: "TSLA".into(),
            name: "Tesla Inc.".into(),
            display: "TSLA".into(),
            price: Some(280.0),
            change: Some(4.5),
        };
        let mut contexts: NewsEntityContexts = HashMap::new();
        contexts.insert("c1".to_string(), vec!["TSLA".to_string(), "tesla".into()]);
        let news_topics: HashMap<String, f64> = HashMap::new();
        let entity_index = NoEntityIndex;
        let mut deduper = InMemoryDeduper::default();
        let clock = FixedClock(ts(2026, 5, 4, 12, 0));
        let signals = detect_market_moves(
            &[market],
            &news_topics,
            &contexts,
            &entity_index,
            &mut deduper,
            &clock,
        );
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_type, SignalType::ExplainedMarketMove);
        assert_eq!(signals[0].data.correlated_entities, vec!["TSLA"]);
        assert_eq!(signals[0].data.correlated_news, vec!["c1"]);
    }

    #[test]
    fn market_move_skipped_below_threshold() {
        let market = MarketData {
            symbol: "TSLA".into(),
            name: "Tesla Inc.".into(),
            display: "TSLA".into(),
            price: Some(280.0),
            change: Some(0.5), // below MARKET_MOVE_THRESHOLD = 2
        };
        let signals = detect_market_moves(
            &[market],
            &HashMap::new(),
            &HashMap::new(),
            &NoEntityIndex,
            &mut InMemoryDeduper::default(),
            &FixedClock(ts(2026, 5, 4, 12, 0)),
        );
        assert!(signals.is_empty());
    }

    // ── flow_price_divergence ───────────────────────────────────

    #[test]
    fn flow_price_divergence_emits_for_energy_symbol_above_threshold() {
        let market = MarketData {
            symbol: "CL=F".into(),
            name: "Crude Oil".into(),
            display: "WTI".into(),
            price: Some(78.0),
            change: Some(2.5),
        };
        let news_topics: HashMap<String, f64> = HashMap::new();
        let signals = detect_flow_price_divergence(
            &[market],
            &news_topics,
            0,
            &mut InMemoryDeduper::default(),
            &FixedClock(ts(2026, 5, 4, 12, 0)),
        );
        assert_eq!(signals.len(), 1);
        assert_eq!(signals[0].signal_type, SignalType::FlowPriceDivergence);
    }

    #[test]
    fn flow_price_divergence_skips_when_pipeline_signals_already_fired() {
        let market = MarketData {
            symbol: "CL=F".into(),
            name: "Crude Oil".into(),
            display: "WTI".into(),
            price: Some(78.0),
            change: Some(3.0),
        };
        let signals = detect_flow_price_divergence(
            &[market],
            &HashMap::new(),
            1,
            &mut InMemoryDeduper::default(),
            &FixedClock(ts(2026, 5, 4, 12, 0)),
        );
        assert!(signals.is_empty());
    }

    #[test]
    fn flow_price_divergence_skips_non_energy_symbol() {
        let market = MarketData {
            symbol: "TSLA".into(),
            name: "Tesla".into(),
            display: "TSLA".into(),
            price: Some(280.0),
            change: Some(5.0),
        };
        let signals = detect_flow_price_divergence(
            &[market],
            &HashMap::new(),
            0,
            &mut InMemoryDeduper::default(),
            &FixedClock(ts(2026, 5, 4, 12, 0)),
        );
        assert!(signals.is_empty());
    }

    // ── orchestrator ────────────────────────────────────────────

    #[test]
    fn analyze_correlations_cold_start_returns_no_signals_but_snapshot_built() {
        let now = ts(2026, 5, 4, 12, 0);
        let event = cluster(
            "c1",
            "Iran tensions rise as Tehran issues warning",
            vec![news_item(
                "Iran tensions rise as Tehran issues warning",
                "Reuters",
                now,
            )],
        );
        let typer = typer_for(&[("Reuters", SourceType::Wire)]);
        let result = analyze_correlations_core(
            &[event],
            &[],
            &[],
            None,
            &typer,
            &NoEntityIndex,
            &HashMap::new(),
            &mut InMemoryDeduper::default(),
            &FixedClock(now),
        );
        assert!(result.signals.is_empty());
        assert!(!result.snapshot.topic_velocity_history.is_empty());
        // Iran is a TOPIC_KEYWORDS entry → velocity counted.
        let iran_history = result.snapshot.topic_velocity_history.get("iran").unwrap();
        assert_eq!(iran_history.len(), 1);
    }

    #[test]
    fn analyze_correlations_filters_below_confidence_threshold() {
        // Construct a single event triggering pipeline_flow_drops
        // with source_count = 1 → confidence = 0.4 + 1/10 = 0.5
        // < 0.6 → filtered out.
        let event = cluster(
            "c1",
            "Pipeline rupture halts flow at terminal",
            vec![news_item(
                "Pipeline rupture halts flow at terminal",
                "Reuters",
                ts(2026, 5, 4, 11, 0),
            )],
        );
        let prev = StreamSnapshot::default();
        let typer = typer_for(&[("Reuters", SourceType::Wire)]);
        let result = analyze_correlations_core(
            &[event],
            &[],
            &[],
            Some(&prev),
            &typer,
            &NoEntityIndex,
            &HashMap::new(),
            &mut InMemoryDeduper::default(),
            &FixedClock(ts(2026, 5, 4, 12, 0)),
        );
        assert!(
            result.signals.is_empty(),
            "low-confidence signal must be dropped, got {:?}",
            result.signals
        );
    }

    #[test]
    fn analyze_correlations_builds_per_market_snapshot_change_map() {
        let market = MarketData {
            symbol: "CL=F".into(),
            name: "Crude Oil".into(),
            display: "WTI".into(),
            price: Some(78.0),
            change: Some(1.7),
        };
        let typer = typer_for(&[]);
        let res = analyze_correlations_core(
            &[],
            &[],
            &[market],
            None,
            &typer,
            &NoEntityIndex,
            &HashMap::new(),
            &mut InMemoryDeduper::default(),
            &FixedClock(ts(2026, 5, 4, 12, 0)),
        );
        let recorded = res.snapshot.market_changes.get("CL=F").copied();
        assert!((recorded.unwrap() - 1.7).abs() < 1e-9);
    }

    #[test]
    fn analyze_correlations_dedupes_signals_by_type() {
        // Two events that would each fire a flow_drop signal with
        // confidence >= 0.6 — orchestrator must keep only one
        // because the dedupe-by-type pass collapses repeats.
        let event_a = cluster(
            "evt-a",
            "Pipeline rupture halts northbound flow at terminal",
            (0..6)
                .map(|i| {
                    news_item(
                        "Pipeline rupture halts northbound flow at terminal",
                        &format!("S{i}"),
                        ts(2026, 5, 4, 11, i),
                    )
                })
                .collect(),
        );
        let event_b = cluster(
            "evt-b",
            "Pipeline outage halts flow northbound terminal again",
            (0..6)
                .map(|i| {
                    news_item(
                        "Pipeline outage halts flow northbound terminal again",
                        &format!("T{i}"),
                        ts(2026, 5, 4, 11, i),
                    )
                })
                .collect(),
        );
        let typer = typer_for(&[]);
        let result = analyze_correlations_core(
            &[event_a, event_b],
            &[],
            &[],
            Some(&StreamSnapshot::default()),
            &typer,
            &NoEntityIndex,
            &HashMap::new(),
            &mut InMemoryDeduper::default(),
            &FixedClock(ts(2026, 5, 4, 12, 0)),
        );
        let flow_drops = result
            .signals
            .iter()
            .filter(|s| s.signal_type == SignalType::FlowDrop)
            .count();
        assert_eq!(flow_drops, 1, "dedupe-by-type kept exactly one flow_drop");
    }
}
