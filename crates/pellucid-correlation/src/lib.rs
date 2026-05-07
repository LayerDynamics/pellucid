//! pellucid-correlation
//!
//! Cross-domain correlation engine — military / escalation / economic
//! / disaster adapters plus the Jaccard clustering primitives shared
//! with the seed pipeline. Ports the algorithms in
//! `worldmonitor/src/services/analysis-core.ts`,
//! `worldmonitor/src/services/correlation-engine/*`, and
//! `worldmonitor/scripts/_clustering.mjs` to Rust.
//!
//! ## Modules shipped in this slice
//!
//! - [`analysis_constants`] — thresholds, stopword set, suppressed
//!   trending terms, topic mappings, and the pure utility functions
//!   `tokenize`, `jaccard_similarity`, `includes_keyword`,
//!   `contains_topic_keyword`, `find_related_topics`,
//!   `generate_signal_id`, `generate_dedupe_key`. Source:
//!   `worldmonitor/src/utils/analysis-constants.ts`.
//! - [`clustering`] — `cluster_items`, `score_importance`,
//!   `select_top_stories`. Single source of truth for the digest /
//!   seeder pipelines. Source: `worldmonitor/scripts/_clustering.mjs`.
//!
//! ## Modules planned for follow-up sessions
//!
//! - `engine` — cross-domain `CorrelationEngine` with the four
//!   `Correlator` adapters (`Military`, `Escalation`, `Economic`,
//!   `Disaster`), grid-based proximity union-find, score weighting,
//!   trend detection, LLM assessment queue. Source:
//!   `worldmonitor/src/services/correlation-engine/{engine,types}.ts`
//!   and `adapters/*.ts`.
//! - `signals` — port of the signal detectors in
//!   `worldmonitor/src/services/analysis-core.ts`
//!   (`detectPipelineFlowDrops`, `detectConvergence`,
//!   `detectTriangulation`, `analyzeCorrelationsCore`).
//! - `news_clustering` — port of `clusterNewsCore` from
//!   `analysis-core.ts` (the threat-aggregating, geo-aware variant
//!   used by the webview worker), distinct from
//!   [`clustering::cluster_items`] which mirrors the simpler
//!   `_clustering.mjs` digest path.

pub mod analysis_constants;
pub mod clustering;
pub mod news_clustering;
pub mod signals;

pub use analysis_constants::{
    contains_topic_keyword, find_related_topics, generate_dedupe_key, generate_signal_id,
    includes_keyword, jaccard_similarity, tokenize, ENERGY_COMMODITY_SYMBOLS, FLOW_DROP_KEYWORDS,
    FLOW_PRICE_THRESHOLD, MARKET_MOVE_THRESHOLD, NEWS_VELOCITY_THRESHOLD, PIPELINE_KEYWORDS,
    PREDICTION_SHIFT_THRESHOLD, SIMILARITY_THRESHOLD, STOP_WORDS, SUPPRESSED_TRENDING_TERMS,
    TOPIC_KEYWORDS, TOPIC_MAPPINGS,
};
pub use clustering::{
    cluster_items, score_importance, select_top_stories, ClusteredNews, NewsItem,
    ThreatClassification, TopStory,
};
pub use news_clustering::{
    aggregate_threats, cluster_news_core, ClusterVelocity, ClusteredEvent, NewsItemCore,
    TierResolver, TopSource,
};
pub use signals::{
    analyze_correlations_core, detect_convergence, detect_flow_price_divergence,
    detect_market_moves, detect_pipeline_flow_drops, detect_prediction_shifts,
    detect_triangulation, detect_velocity_spikes, AnalyzeResult, Clock, CorrelationSignal,
    EntityIndex, FixedClock, InMemoryDeduper, MarketData, NewsEntityContexts, NoEntityIndex,
    PredictionMarket, SignalData, SignalDeduper, SignalType, SourceType, SourceTyper,
    StreamSnapshot, SystemClock, TopicVelocityPoint,
};

/// Returns the crate version string from `CARGO_PKG_VERSION`.
#[must_use]
pub fn version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn version_is_set() {
        let v = version();
        assert!(!v.is_empty(), "version must not be empty");
        assert!(v.contains('.'), "expected semver with dot, got {v}");
    }

    #[test]
    fn version_matches_workspace() {
        assert_eq!(version(), env!("CARGO_PKG_VERSION"));
    }
}
