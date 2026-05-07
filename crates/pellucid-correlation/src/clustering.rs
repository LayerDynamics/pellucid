//! Server-side clustering — port of
//! `worldmonitor/scripts/_clustering.mjs`.
//!
//! Three public functions:
//! - [`cluster_items`] — Jaccard inverted-index clustering with a
//!   tier-priority primary pick. Single source of truth for digest
//!   and seeder pipelines.
//! - [`score_importance`] — keyword-weighted news importance score.
//!   Higher scores rank breaking-violence/conflict/flashpoint stories
//!   above business news.
//! - [`select_top_stories`] — pick top-N most-important stories with
//!   a per-source diversity cap (default 3 per source).
//!
//! All three are pure functions and match the original JavaScript
//! 1:1; the parity tests at the bottom of this file mirror every
//! assertion in `worldmonitor/tests/clustering.test.mjs`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::analysis_constants::{jaccard_similarity, tokenize, SIMILARITY_THRESHOLD};

// ============================================================================
// Public types
// ============================================================================

/// Threat classification carried on input items and (sometimes)
/// promoted onto cluster output. Mirrors the `ThreatClassification`
/// shape the JS source carries on its news items.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThreatClassification {
    /// Severity level — `critical` / `high` / `medium` / `low` /
    /// `info`. Stored as a `String` so the field is forward
    /// compatible with future levels without recompiling consumers.
    pub level: String,
    /// Origin of the classification — `keyword` (heuristic) or one
    /// of the model identifiers (`bart`, `xlm-roberta`, …). The
    /// promotion rule in [`cluster_items`] picks the first
    /// non-`keyword` entry it finds.
    pub source: String,
    /// Optional category bucket (`military`, `disaster`, etc).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    /// Optional confidence in `[0, 1]`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
}

/// News item input shape consumed by [`cluster_items`]. Field names
/// are camelCase'd via `serde(rename)` so JSON payloads from the
/// existing `_clustering.mjs` test fixtures parse without
/// preprocessing.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewsItem {
    pub title: String,
    pub source: String,
    pub link: String,
    /// Optional ISO-8601 publication timestamp. Used as a tie-breaker
    /// in primary-source selection (newer wins when tiers tie).
    #[serde(default, rename = "pubDate", skip_serializing_if = "Option::is_none")]
    pub pub_date: Option<DateTime<Utc>>,
    /// Optional source-tier rank — lower is more authoritative.
    /// Defaults to 99 in the sort (matches `_clustering.mjs:117`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<u32>,
    /// `true` if the upstream feed flagged this item as a breaking
    /// alert.
    #[serde(default, rename = "isAlert")]
    pub is_alert: bool,
    /// Optional threat classification carried through clustering.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threat: Option<ThreatClassification>,
}

/// Output of [`cluster_items`]. One entry per cluster; `source_count`
/// is the number of items merged into the cluster.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClusteredNews {
    #[serde(rename = "primaryTitle")]
    pub primary_title: String,
    #[serde(rename = "primarySource")]
    pub primary_source: String,
    #[serde(rename = "primaryLink")]
    pub primary_link: String,
    #[serde(default, rename = "pubDate", skip_serializing_if = "Option::is_none")]
    pub pub_date: Option<DateTime<Utc>>,
    #[serde(rename = "sourceCount")]
    pub source_count: usize,
    #[serde(rename = "isAlert")]
    pub is_alert: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threat: Option<ThreatClassification>,
}

/// One entry returned by [`select_top_stories`] — the cluster plus
/// its computed importance score. The JS source spreads
/// `{...cluster, importanceScore: score}` into a single object;
/// keeping a wrapper here makes it explicit which field came from
/// where.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopStory {
    #[serde(flatten)]
    pub cluster: ClusteredNews,
    #[serde(rename = "importanceScore")]
    pub importance_score: f64,
}

// ============================================================================
// Constants — keyword sets used by `score_importance`
// (lifted verbatim from `_clustering.mjs:17-50`)
// ============================================================================

const MILITARY_KEYWORDS: &[&str] = &[
    "war",
    "armada",
    "invasion",
    "airstrike",
    "strike",
    "missile",
    "troops",
    "deployed",
    "offensive",
    "artillery",
    "bomb",
    "combat",
    "fleet",
    "warship",
    "carrier",
    "navy",
    "airforce",
    "deployment",
    "mobilization",
    "attack",
];

const VIOLENCE_KEYWORDS: &[&str] = &[
    "killed",
    "dead",
    "death",
    "shot",
    "blood",
    "massacre",
    "slaughter",
    "fatalities",
    "casualties",
    "wounded",
    "injured",
    "murdered",
    "execution",
    "crackdown",
    "violent",
    "clashes",
    "gunfire",
    "shooting",
];

const UNREST_KEYWORDS: &[&str] = &[
    "protest",
    "protests",
    "uprising",
    "revolt",
    "revolution",
    "riot",
    "riots",
    "demonstration",
    "unrest",
    "dissent",
    "rebellion",
    "insurgent",
    "overthrow",
    "coup",
    "martial law",
    "curfew",
    "shutdown",
    "blackout",
];

const FLASHPOINT_KEYWORDS: &[&str] = &[
    "iran",
    "tehran",
    "russia",
    "moscow",
    "china",
    "beijing",
    "taiwan",
    "ukraine",
    "kyiv",
    "north korea",
    "pyongyang",
    "israel",
    "gaza",
    "west bank",
    "syria",
    "damascus",
    "yemen",
    "hezbollah",
    "hamas",
    "kremlin",
    "pentagon",
    "nato",
    "wagner",
];

const CRISIS_KEYWORDS: &[&str] = &[
    "crisis",
    "emergency",
    "catastrophe",
    "disaster",
    "collapse",
    "humanitarian",
    "sanctions",
    "ultimatum",
    "threat",
    "retaliation",
    "escalation",
    "tensions",
    "breaking",
    "urgent",
    "developing",
    "exclusive",
];

const DEMOTE_KEYWORDS: &[&str] = &[
    "ceo",
    "earnings",
    "stock",
    "startup",
    "data center",
    "datacenter",
    "revenue",
    "quarterly",
    "profit",
    "investor",
    "ipo",
    "funding",
    "valuation",
];

/// Per-source cap for [`select_top_stories`] — matches the
/// `MAX_PER_SOURCE = 3` constant at `_clustering.mjs:188`.
const MAX_PER_SOURCE: usize = 3;

// ============================================================================
// Sort helpers
// ============================================================================

/// JavaScript-compatible tier default used when `tier` is missing
/// (matches `(a.tier ?? 99)` at `_clustering.mjs:117`).
const MISSING_TIER: u32 = 99;

fn count_matches(text: &str, keywords: &[&str]) -> u32 {
    let mut n = 0u32;
    for kw in keywords {
        if text.contains(kw) {
            n += 1;
        }
    }
    n
}

// ============================================================================
// `cluster_items`
// ============================================================================

/// Jaccard inverted-index clustering. For each item `i`, intersect
/// the inverted-index buckets of each token to find candidate
/// indices `j > i`, then accept `j` into `i`'s cluster when the
/// Jaccard similarity of their token sets ≥ [`SIMILARITY_THRESHOLD`].
///
/// Within each cluster the primary source is picked by ascending
/// tier (more authoritative wins), with descending publication date
/// as the tie-breaker. The cluster's threat is the first non-`keyword`
/// classification found in the tier-sorted order, falling back to
/// the primary item's own threat.
///
/// Mirrors `_clustering.mjs:71-135` exactly.
#[must_use]
pub fn cluster_items(items: &[NewsItem]) -> Vec<ClusteredNews> {
    if items.is_empty() {
        return Vec::new();
    }

    let token_list: Vec<std::collections::HashSet<String>> =
        items.iter().map(|item| tokenize(&item.title)).collect();

    // Build inverted index — token → list of item indices in
    // ascending order.
    let mut inverted_index: std::collections::HashMap<String, Vec<usize>> =
        std::collections::HashMap::new();
    for (i, tokens) in token_list.iter().enumerate() {
        for token in tokens {
            inverted_index.entry(token.clone()).or_default().push(i);
        }
    }

    let mut clusters: Vec<Vec<usize>> = Vec::new();
    let mut assigned: Vec<bool> = vec![false; items.len()];

    for i in 0..items.len() {
        if assigned[i] {
            continue;
        }
        let mut cluster: Vec<usize> = vec![i];
        assigned[i] = true;
        let tokens_i = &token_list[i];

        // Collect unique candidate indices > i from the inverted
        // index. The JS source uses `Set` + sort; we use `BTreeSet`
        // which gives sorted iteration directly.
        let mut candidates: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
        for token in tokens_i {
            if let Some(bucket) = inverted_index.get(token) {
                for &idx in bucket {
                    if idx > i {
                        candidates.insert(idx);
                    }
                }
            }
        }

        for j in candidates {
            if assigned[j] {
                continue;
            }
            if jaccard_similarity(tokens_i, &token_list[j]) >= SIMILARITY_THRESHOLD {
                cluster.push(j);
                assigned[j] = true;
            }
        }

        clusters.push(cluster);
    }

    clusters
        .into_iter()
        .map(|group_idx| {
            // Tier-then-recency sort (low tier first, then newer
            // pubDate first). Mirrors `_clustering.mjs:116-120`.
            let mut sorted: Vec<&NewsItem> = group_idx.iter().map(|&idx| &items[idx]).collect();
            sorted.sort_by(|a, b| {
                let ta = a.tier.unwrap_or(MISSING_TIER);
                let tb = b.tier.unwrap_or(MISSING_TIER);
                let by_tier = ta.cmp(&tb);
                if by_tier != std::cmp::Ordering::Equal {
                    return by_tier;
                }
                // pubDate descending — when both Some, larger
                // timestamp comes first; missing dates compare as
                // None which sorts as the smallest value, putting
                // dated items first (matches JS `new Date(undefined)
                // → NaN` which yields a stable comparator).
                match (a.pub_date, b.pub_date) {
                    (Some(ad), Some(bd)) => bd.cmp(&ad),
                    (Some(_), None) => std::cmp::Ordering::Less,
                    (None, Some(_)) => std::cmp::Ordering::Greater,
                    (None, None) => std::cmp::Ordering::Equal,
                }
            });

            let primary = sorted[0];

            // Threat promotion — first non-`keyword` source in the
            // tier-sorted list wins; otherwise fall back to the
            // primary's threat. Mirrors `_clustering.mjs:124-133`.
            let promoted = sorted
                .iter()
                .find(|i| {
                    i.threat
                        .as_ref()
                        .is_some_and(|t| !t.level.is_empty() && t.source != "keyword")
                })
                .and_then(|i| i.threat.clone());
            let threat = promoted.or_else(|| primary.threat.clone());

            let is_alert = group_idx.iter().any(|&idx| items[idx].is_alert);

            ClusteredNews {
                primary_title: primary.title.clone(),
                primary_source: primary.source.clone(),
                primary_link: primary.link.clone(),
                pub_date: primary.pub_date,
                source_count: group_idx.len(),
                is_alert,
                threat,
            }
        })
        .collect()
}

// ============================================================================
// `score_importance`
// ============================================================================

/// Keyword-weighted importance scorer. Mirrors
/// `_clustering.mjs:141-172` constant-for-constant.
///
/// Score components (bigger contributes more):
/// - `source_count * 10`
/// - violence keywords: `100 + n * 25`
/// - military keywords: `80 + n * 20`
/// - unrest keywords:   `70 + n * 18`
/// - flashpoint:        `60 + n * 15`
/// - **combo bonus** ×1.5 when (violence OR unrest) AND flashpoint
/// - crisis keywords:   `30 + n * 10`
/// - **demote** ×0.3 when business keywords present
/// - alert: +50
#[must_use]
pub fn score_importance(cluster: &ClusteredNews) -> f64 {
    let mut score = 0.0_f64;
    let title_lower = cluster.primary_title.to_lowercase();

    // Source count contribution. JS uses `cluster.sourceCount || 1`
    // which treats 0 as 1; in Rust the type is `usize` so we mirror
    // that defaulting behaviour explicitly.
    let source_count_eff = if cluster.source_count == 0 {
        1
    } else {
        cluster.source_count
    };
    score += (source_count_eff as f64) * 10.0;

    let violence_n = count_matches(&title_lower, VIOLENCE_KEYWORDS);
    if violence_n > 0 {
        score += 100.0 + f64::from(violence_n) * 25.0;
    }

    let military_n = count_matches(&title_lower, MILITARY_KEYWORDS);
    if military_n > 0 {
        score += 80.0 + f64::from(military_n) * 20.0;
    }

    let unrest_n = count_matches(&title_lower, UNREST_KEYWORDS);
    if unrest_n > 0 {
        score += 70.0 + f64::from(unrest_n) * 18.0;
    }

    let flashpoint_n = count_matches(&title_lower, FLASHPOINT_KEYWORDS);
    if flashpoint_n > 0 {
        score += 60.0 + f64::from(flashpoint_n) * 15.0;
    }

    if (violence_n > 0 || unrest_n > 0) && flashpoint_n > 0 {
        score *= 1.5;
    }

    let crisis_n = count_matches(&title_lower, CRISIS_KEYWORDS);
    if crisis_n > 0 {
        score += 30.0 + f64::from(crisis_n) * 10.0;
    }

    let demote_n = count_matches(&title_lower, DEMOTE_KEYWORDS);
    if demote_n > 0 {
        score *= 0.3;
    }

    if cluster.is_alert {
        score += 50.0;
    }

    score
}

// ============================================================================
// `select_top_stories`
// ============================================================================

/// Filter clusters to the most important ones, then return the top
/// `max_count` by score with a per-source cap of [`MAX_PER_SOURCE`]
/// (3). Filter passes when **any** of: ≥ 2 sources, alerted, or score
/// > 100 — mirrors `_clustering.mjs:176-201`.
#[must_use]
pub fn select_top_stories(clusters: &[ClusteredNews], max_count: usize) -> Vec<TopStory> {
    // Score + filter.
    let mut scored: Vec<(ClusteredNews, f64)> = clusters
        .iter()
        .map(|c| (c.clone(), score_importance(c)))
        .filter(|(c, score)| {
            let count = if c.source_count == 0 {
                1
            } else {
                c.source_count
            };
            count >= 2 || c.is_alert || *score > 100.0
        })
        .collect();

    // Sort by score descending. JS sorts with `b.score - a.score`
    // which is a stable sort in V8 (Timsort). Rust's `sort_by`
    // is also stable, so identical-score entries keep input order.
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));

    let mut selected: Vec<TopStory> = Vec::new();
    let mut source_count: std::collections::HashMap<String, usize> =
        std::collections::HashMap::new();
    for (cluster, score) in scored {
        let count = source_count
            .get(&cluster.primary_source)
            .copied()
            .unwrap_or(0);
        if count < MAX_PER_SOURCE {
            source_count.insert(cluster.primary_source.clone(), count + 1);
            selected.push(TopStory {
                cluster,
                importance_score: score,
            });
        }
        if selected.len() >= max_count {
            break;
        }
    }
    selected
}

// ============================================================================
// Tests — mirror `worldmonitor/tests/clustering.test.mjs` 1:1
// ============================================================================

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    fn item(title: &str, source: &str, link: &str) -> NewsItem {
        NewsItem {
            title: title.into(),
            source: source.into(),
            link: link.into(),
            pub_date: None,
            tier: None,
            is_alert: false,
            threat: None,
        }
    }

    fn item_with_tier(title: &str, source: &str, link: &str, tier: u32) -> NewsItem {
        NewsItem {
            tier: Some(tier),
            ..item(title, source, link)
        }
    }

    fn cluster_with(title: &str, source_count: usize) -> ClusteredNews {
        ClusteredNews {
            primary_title: title.into(),
            primary_source: "Reuters".into(),
            primary_link: "http://x".into(),
            pub_date: None,
            source_count,
            is_alert: false,
            threat: None,
        }
    }

    fn cluster_full(title: &str, source: &str, link: &str, source_count: usize) -> ClusteredNews {
        ClusteredNews {
            primary_title: title.into(),
            primary_source: source.into(),
            primary_link: link.into(),
            pub_date: None,
            source_count,
            is_alert: false,
            threat: None,
        }
    }

    // ── clusterItems ───────────────────────────────────────────────

    #[test]
    fn cluster_items_groups_similar_titles_into_one_cluster() {
        // clustering.test.mjs:7-15
        let items = vec![
            item(
                "Iran launches missile strikes on targets in Syria overnight",
                "Reuters",
                "http://a",
            ),
            item(
                "Iran launches missile strikes on targets in Syria overnight says officials",
                "AP",
                "http://b",
            ),
        ];
        let clusters = cluster_items(&items);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].source_count, 2);
    }

    #[test]
    fn cluster_items_keeps_different_titles_as_separate_clusters() {
        // clustering.test.mjs:17-24
        let items = vec![
            item(
                "Iran launches missile strikes on targets in Syria",
                "Reuters",
                "http://a",
            ),
            item(
                "Stock market rallies on tech earnings report",
                "CNBC",
                "http://b",
            ),
        ];
        let clusters = cluster_items(&items);
        assert_eq!(clusters.len(), 2);
    }

    #[test]
    fn cluster_items_returns_empty_array_for_empty_input() {
        // clustering.test.mjs:26-28
        assert_eq!(cluster_items(&[]), Vec::<ClusteredNews>::new());
    }

    #[test]
    fn cluster_items_preserves_primary_title_from_highest_tier_source() {
        // clustering.test.mjs:30-38
        let items = vec![
            item_with_tier("Iran strikes Syria overnight", "Blog", "http://b", 5),
            item_with_tier(
                "Iran strikes Syria overnight confirms officials",
                "Reuters",
                "http://a",
                1,
            ),
        ];
        let clusters = cluster_items(&items);
        assert_eq!(clusters.len(), 1);
        assert_eq!(clusters[0].primary_source, "Reuters");
    }

    // ── scoreImportance ────────────────────────────────────────────

    #[test]
    fn score_importance_military_higher_than_business() {
        // clustering.test.mjs:42-46
        let military = cluster_with("Troops deployed after missile attack in Ukraine", 2);
        let business = cluster_with("Tech startup raises funding in quarterly earnings", 2);
        assert!(score_importance(&military) > score_importance(&business));
    }

    #[test]
    fn score_importance_combo_bonus_for_flashpoint_plus_violence() {
        // clustering.test.mjs:48-52
        let flashpoint_violence =
            cluster_with("Iran crackdown killed dozens in Tehran protests", 1);
        let violence_only = cluster_with("Crackdown killed dozens in protests", 1);
        assert!(score_importance(&flashpoint_violence) > score_importance(&violence_only));
    }

    #[test]
    fn score_importance_demotes_business_context() {
        // clustering.test.mjs:54-58
        let pure = cluster_with("Strike hits military targets", 1);
        let business = cluster_with("Strike hits military targets says CEO in earnings call", 1);
        assert!(score_importance(&pure) > score_importance(&business));
    }

    #[test]
    fn score_importance_adds_alert_bonus() {
        // clustering.test.mjs:60-64
        let mut no_alert = cluster_with("Earthquake hits region", 1);
        no_alert.is_alert = false;
        let mut alert = cluster_with("Earthquake hits region", 1);
        alert.is_alert = true;
        assert!(score_importance(&alert) > score_importance(&no_alert));
    }

    // ── selectTopStories ───────────────────────────────────────────

    #[test]
    fn select_top_stories_returns_at_most_max_count() {
        // clustering.test.mjs:68-78
        let clusters: Vec<ClusteredNews> = (0..20)
            .map(|i| {
                cluster_full(
                    &format!("War conflict attack story number {i}"),
                    &format!("Source{}", i % 5),
                    &format!("http://{i}"),
                    3,
                )
            })
            .collect();
        let top = select_top_stories(&clusters, 5);
        assert!(top.len() <= 5);
    }

    #[test]
    fn select_top_stories_filters_low_scoring_single_source_non_alert() {
        // clustering.test.mjs:80-86
        let clusters = vec![cluster_full("Nice weather today", "Blog", "http://a", 1)];
        let top = select_top_stories(&clusters, 8);
        assert_eq!(top.len(), 0);
    }

    #[test]
    fn select_top_stories_includes_high_scoring_single_source() {
        // clustering.test.mjs:88-94
        let clusters = vec![cluster_full(
            "Iran missile attack kills dozens in massive airstrike",
            "Reuters",
            "http://a",
            1,
        )];
        let top = select_top_stories(&clusters, 8);
        assert_eq!(top.len(), 1);
    }

    #[test]
    fn select_top_stories_limits_per_source_diversity() {
        // clustering.test.mjs:96-106
        let clusters: Vec<ClusteredNews> = (0..10)
            .map(|i| {
                cluster_full(
                    &format!("War attack missile strike story {i}"),
                    "SameSource",
                    &format!("http://{i}"),
                    2,
                )
            })
            .collect();
        let top = select_top_stories(&clusters, 8);
        assert!(top.len() <= 3);
    }

    // ── extra parity tests not in the JS file ──────────────────────

    #[test]
    fn cluster_items_threat_promotion_prefers_non_keyword_source() {
        // The first non-`keyword`-source threat in tier order wins
        // (`_clustering.mjs:124`). Two items in one cluster: the
        // higher-tier one has a `keyword`-source threat, the lower-
        // tier one has a `bart`-source threat. Promotion picks the
        // `bart` threat.
        let high_tier = NewsItem {
            tier: Some(1),
            threat: Some(ThreatClassification {
                level: "medium".into(),
                source: "keyword".into(),
                category: None,
                confidence: None,
            }),
            ..item("Iran strikes Syria overnight", "Reuters", "http://a")
        };
        let low_tier = NewsItem {
            tier: Some(5),
            threat: Some(ThreatClassification {
                level: "high".into(),
                source: "bart".into(),
                category: Some("military".into()),
                confidence: Some(0.9),
            }),
            ..item("Iran strikes Syria overnight again", "Blog", "http://b")
        };
        let clusters = cluster_items(&[high_tier, low_tier]);
        assert_eq!(clusters.len(), 1);
        let promoted = clusters[0].threat.as_ref().unwrap();
        assert_eq!(promoted.source, "bart");
        assert_eq!(promoted.level, "high");
    }

    #[test]
    fn cluster_items_alert_propagates_when_any_member_alerted() {
        let mut a = item("Iran strikes Syria", "Reuters", "http://a");
        a.is_alert = false;
        let mut b = item("Iran strikes Syria overnight", "AP", "http://b");
        b.is_alert = true;
        let clusters = cluster_items(&[a, b]);
        assert_eq!(clusters.len(), 1);
        assert!(clusters[0].is_alert);
    }

    #[test]
    fn select_top_stories_sorts_by_score_descending() {
        let clusters = vec![
            cluster_full("Stock startup earnings", "X", "http://x", 2), // demoted business
            cluster_full(
                "Iran missile strike kills dozens in Tehran",
                "Reuters",
                "http://r",
                3,
            ), // big score
            cluster_full("Some routine update", "Y", "http://y", 2),
        ];
        let top = select_top_stories(&clusters, 8);
        // The military/flashpoint/violence story must be first.
        assert!(top[0].cluster.primary_source == "Reuters");
        // Importance score is monotonically non-increasing.
        for w in top.windows(2) {
            assert!(w[0].importance_score >= w[1].importance_score);
        }
    }
}
