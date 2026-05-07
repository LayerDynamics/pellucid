//! Webview-side news clustering — port of
//! `worldmonitor/src/services/analysis-core.ts::clusterNewsCore`
//! and the surrounding `aggregateThreats` helper.
//!
//! Produces a richer cluster shape than [`crate::clustering`]
//! (the digest variant), specifically:
//!
//! - aggregated [`ThreatClassification`] across cluster members,
//!   weighted by source tier;
//! - the modal `(lat, lon)` of geolocated members, so the cluster
//!   inherits a single representative point;
//! - the top-3 source list (tier-sorted);
//! - first-seen / last-updated timestamps;
//! - a deterministic `id` derived from the earliest item's pubDate
//!   + first 20 alphanumeric characters of its title.
//!
//! The `Vec<ClusteredEvent>` returned is sorted by `last_updated`
//! descending so callers can drop the head N entries straight into a
//! UI feed.

use std::collections::HashMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::analysis_constants::{jaccard_similarity, tokenize, SIMILARITY_THRESHOLD};
use crate::clustering::ThreatClassification;

// ============================================================================
// Public types — mirror `analysis-core.ts:103-137`
// ============================================================================

/// News item input shape for [`cluster_news_core`].
///
/// Distinct from [`crate::clustering::NewsItem`] (the digest input)
/// in that:
/// - `pub_date` is **required** here (the algorithm orders members
///   by pubDate descending after tier ordering, and the cluster id
///   is derived from the earliest member's pubDate);
/// - geo fields (`lat`, `lon`, `location_name`) and `lang` are
///   carried through so the produced cluster can inherit them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewsItemCore {
    pub source: String,
    pub title: String,
    pub link: String,
    #[serde(rename = "pubDate")]
    pub pub_date: DateTime<Utc>,
    #[serde(rename = "isAlert")]
    pub is_alert: bool,
    #[serde(
        default,
        rename = "monitorColor",
        skip_serializing_if = "Option::is_none"
    )]
    pub monitor_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tier: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threat: Option<ThreatClassification>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lat: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lon: Option<f64>,
    #[serde(
        default,
        rename = "locationName",
        skip_serializing_if = "Option::is_none"
    )]
    pub location_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
}

/// Top-3 source row carried on the cluster output.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TopSource {
    pub name: String,
    pub tier: u32,
    pub url: String,
}

/// Velocity statistic — present when computed by the velocity-spike
/// detector or the upstream feed scorer. The webview reads
/// `sources_per_hour` to render the "rising" indicator on a panel.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct ClusterVelocity {
    #[serde(
        default,
        rename = "sourcesPerHour",
        skip_serializing_if = "Option::is_none"
    )]
    pub sources_per_hour: Option<f64>,
}

/// Webview cluster shape returned by [`cluster_news_core`]. Mirrors
/// `ClusteredEventCore` (`analysis-core.ts:120-137`) field-for-field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClusteredEvent {
    pub id: String,
    #[serde(rename = "primaryTitle")]
    pub primary_title: String,
    #[serde(rename = "primarySource")]
    pub primary_source: String,
    #[serde(rename = "primaryLink")]
    pub primary_link: String,
    #[serde(rename = "sourceCount")]
    pub source_count: usize,
    #[serde(rename = "topSources")]
    pub top_sources: Vec<TopSource>,
    #[serde(rename = "allItems")]
    pub all_items: Vec<NewsItemCore>,
    #[serde(rename = "firstSeen")]
    pub first_seen: DateTime<Utc>,
    #[serde(rename = "lastUpdated")]
    pub last_updated: DateTime<Utc>,
    #[serde(rename = "isAlert")]
    pub is_alert: bool,
    #[serde(
        default,
        rename = "monitorColor",
        skip_serializing_if = "Option::is_none"
    )]
    pub monitor_color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub velocity: Option<ClusterVelocity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threat: Option<ThreatClassification>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lat: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lon: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lang: Option<String>,
}

// ============================================================================
// Threat aggregation — port of `analysis-core.ts:39-79`
// ============================================================================

/// Aggregate threat classifications across a cluster's members.
///
/// - **Level**: maximum severity in the cluster (priority
///   `critical > high > medium > low > info`). Items without a
///   threat are skipped entirely; if no member has a threat the
///   default `info / general / 0.3 / keyword` envelope is returned
///   (same as JS).
/// - **Category**: most common category among threatened items.
/// - **Confidence**: tier-weighted mean of confidences — weight =
///   `6 - min(tier, 5)` so tier-1 sources count 5×, tier-5 1×, no-tier
///   1× (the JS uses `tier ? (6 - …) : 1`).
/// - **Source**: always `"keyword"` because aggregation discards the
///   per-item provenance.
#[must_use]
pub fn aggregate_threats(
    items: &[(Option<&ThreatClassification>, Option<u32>)],
) -> ThreatClassification {
    let with_threat: Vec<(&ThreatClassification, Option<u32>)> = items
        .iter()
        .filter_map(|(threat, tier)| threat.map(|t| (t, *tier)))
        .collect();

    if with_threat.is_empty() {
        return ThreatClassification {
            level: "info".into(),
            source: "keyword".into(),
            category: Some("general".into()),
            confidence: Some(0.3),
        };
    }

    let max_level = with_threat
        .iter()
        .map(|(t, _)| t.level.as_str())
        .max_by_key(|s| threat_priority(s))
        .map(str::to_string)
        .unwrap_or_else(|| "info".into());

    let mut cat_counts: HashMap<String, u32> = HashMap::new();
    for (t, _) in &with_threat {
        let cat = t.category.clone().unwrap_or_else(|| "general".into());
        *cat_counts.entry(cat).or_insert(0) += 1;
    }
    let top_category = cat_counts
        .into_iter()
        .max_by_key(|(_, n)| *n)
        .map(|(c, _)| c)
        .unwrap_or_else(|| "general".into());

    let mut weighted_sum = 0.0_f64;
    let mut weight_total = 0.0_f64;
    for (t, tier) in &with_threat {
        let weight = match tier {
            Some(v) => f64::from(6_u32.saturating_sub((*v).min(5))),
            None => 1.0,
        };
        weighted_sum += t.confidence.unwrap_or(0.5) * weight;
        weight_total += weight;
    }
    let confidence = if weight_total > 0.0 {
        weighted_sum / weight_total
    } else {
        0.5
    };

    ThreatClassification {
        level: max_level,
        source: "keyword".into(),
        category: Some(top_category),
        confidence: Some(confidence),
    }
}

fn threat_priority(level: &str) -> u32 {
    match level {
        "critical" => 5,
        "high" => 4,
        "medium" => 3,
        "low" => 2,
        "info" => 1,
        _ => 0,
    }
}

// ============================================================================
// `cluster_news_core` — port of `analysis-core.ts:215-342`
// ============================================================================

/// Tier resolver — maps a source string to its authority tier.
/// Lower values are more authoritative. Matches the JS signature
/// `(source: string) => number`.
pub trait TierResolver {
    fn tier_of(&self, source: &str) -> u32;
}

impl<F> TierResolver for F
where
    F: Fn(&str) -> u32,
{
    fn tier_of(&self, source: &str) -> u32 {
        (self)(source)
    }
}

/// Cluster news items by Jaccard similarity of their titles, using
/// an inverted index for O(n · k) lookup of candidates.
///
/// Mirrors `clusterNewsCore` (`analysis-core.ts:215-342`):
///
/// 1. Tokenize every title; build inverted index.
/// 2. Greedy single-pass clustering — for each unassigned item `i`,
///    walk the union of inverted-index buckets for `i`'s tokens to
///    find candidate `j > i` with similarity ≥
///    [`SIMILARITY_THRESHOLD`].
/// 3. For each cluster, sort members by `(tier ascending,
///    pub_date descending)` and use member 0 as the primary.
/// 4. Aggregate threats, pick modal geo location, take first 3
///    members as `top_sources`.
/// 5. Sort the result by `last_updated` descending.
#[must_use]
pub fn cluster_news_core<R: TierResolver>(
    items: &[NewsItemCore],
    resolver: &R,
) -> Vec<ClusteredEvent> {
    if items.is_empty() {
        return Vec::new();
    }

    // Resolve tiers up front so we can sort by them later.
    let tiers: Vec<u32> = items
        .iter()
        .map(|item| item.tier.unwrap_or_else(|| resolver.tier_of(&item.source)))
        .collect();

    let token_list: Vec<std::collections::HashSet<String>> =
        items.iter().map(|i| tokenize(&i.title)).collect();

    let mut inverted_index: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, tokens) in token_list.iter().enumerate() {
        for token in tokens {
            inverted_index.entry(token.clone()).or_default().push(i);
        }
    }

    let mut clusters: Vec<Vec<usize>> = Vec::new();
    let mut assigned = vec![false; items.len()];

    for i in 0..items.len() {
        if assigned[i] {
            continue;
        }
        let mut group = vec![i];
        assigned[i] = true;
        let tokens_i = &token_list[i];

        // Sorted ascending so the JS iteration order is preserved.
        let mut candidates: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
        for tok in tokens_i {
            if let Some(bucket) = inverted_index.get(tok) {
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
                group.push(j);
                assigned[j] = true;
            }
        }
        clusters.push(group);
    }

    let mut out: Vec<ClusteredEvent> = clusters
        .into_iter()
        .map(|group| build_cluster(group, items, &tiers))
        .collect();
    // last_updated descending
    out.sort_by_key(|c| std::cmp::Reverse(c.last_updated));
    out
}

fn build_cluster(group_idx: Vec<usize>, items: &[NewsItemCore], tiers: &[u32]) -> ClusteredEvent {
    // Sort by (tier asc, pub_date desc)
    let mut sorted: Vec<usize> = group_idx.clone();
    sorted.sort_by(|&a, &b| {
        let ta = tiers[a];
        let tb = tiers[b];
        match ta.cmp(&tb) {
            std::cmp::Ordering::Equal => items[b].pub_date.cmp(&items[a].pub_date),
            other => other,
        }
    });
    let primary_idx = sorted[0];
    let primary = &items[primary_idx];

    // Top 3 sources from tier-sorted order.
    let top_sources: Vec<TopSource> = sorted
        .iter()
        .take(3)
        .map(|&idx| TopSource {
            name: items[idx].source.clone(),
            tier: tiers[idx],
            url: items[idx].link.clone(),
        })
        .collect();

    let first_seen = group_idx
        .iter()
        .map(|&i| items[i].pub_date)
        .min()
        .unwrap_or(primary.pub_date);
    let last_updated = group_idx
        .iter()
        .map(|&i| items[i].pub_date)
        .max()
        .unwrap_or(primary.pub_date);
    let is_alert = group_idx.iter().any(|&i| items[i].is_alert);
    let monitor_color = group_idx
        .iter()
        .find_map(|&i| items[i].monitor_color.clone());

    // Threat aggregation.
    let threat_input: Vec<(Option<&ThreatClassification>, Option<u32>)> = group_idx
        .iter()
        .map(|&i| (items[i].threat.as_ref(), Some(tiers[i])))
        .collect();
    let threat = Some(aggregate_threats(&threat_input));

    // Modal (lat, lon) — the (lat, lon) pair that appears most among
    // members. Ties are broken by HashMap iteration order which is
    // non-deterministic in stdlib; we explicitly pick the
    // smallest-key tie-break by sorting on the string key for
    // determinism.
    let mut loc_counts: HashMap<String, (f64, f64, u32)> = HashMap::new();
    for &i in &group_idx {
        if let (Some(lat), Some(lon)) = (items[i].lat, items[i].lon) {
            let key = format!("{lat},{lon}");
            let entry = loc_counts.entry(key).or_insert((lat, lon, 0));
            entry.2 += 1;
        }
    }
    let (cluster_lat, cluster_lon) = {
        let mut entries: Vec<(String, (f64, f64, u32))> = loc_counts.into_iter().collect();
        // Sort by count desc, then key asc for deterministic tie-break.
        entries.sort_by(|a, b| b.1 .2.cmp(&a.1 .2).then_with(|| a.0.cmp(&b.0)));
        match entries.into_iter().next() {
            Some((_, (lat, lon, _))) => (Some(lat), Some(lon)),
            None => (None, None),
        }
    };

    // All items, in original input order (matches JS `cluster`
    // before sort — `cluster` is built in inverted-index iteration
    // order, which is ascending index).
    let mut all_items: Vec<NewsItemCore> = group_idx.iter().map(|&i| items[i].clone()).collect();
    all_items.sort_by_key(|i| i.pub_date);

    let id = generate_cluster_id(&all_items);

    ClusteredEvent {
        id,
        primary_title: primary.title.clone(),
        primary_source: primary.source.clone(),
        primary_link: primary.link.clone(),
        source_count: group_idx.len(),
        top_sources,
        all_items,
        first_seen,
        last_updated,
        is_alert,
        monitor_color,
        velocity: None,
        threat,
        lat: cluster_lat,
        lon: cluster_lon,
        lang: primary.lang.clone(),
    }
}

/// Mirror of `generateClusterId` (`analysis-core.ts:205-209`):
/// `<earliest_pubdate_ms>-<first 20 alphanumeric chars of earliest title>`.
fn generate_cluster_id(items_sorted_by_date_asc: &[NewsItemCore]) -> String {
    let first = &items_sorted_by_date_asc[0];
    let title_slug: String = first
        .title
        .chars()
        .take(20)
        .filter(|c| c.is_ascii_alphanumeric())
        .collect();
    format!("{}-{}", first.pub_date.timestamp_millis(), title_slug)
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn ts(year: i32, month: u32, day: u32, hour: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(year, month, day, hour, 0, 0).unwrap()
    }

    fn item(title: &str, source: &str, tier: Option<u32>, when: DateTime<Utc>) -> NewsItemCore {
        NewsItemCore {
            source: source.into(),
            title: title.into(),
            link: format!("http://example/{source}"),
            pub_date: when,
            is_alert: false,
            monitor_color: None,
            tier,
            threat: None,
            lat: None,
            lon: None,
            location_name: None,
            lang: None,
        }
    }

    fn const_resolver(_: &str) -> u32 {
        4
    }

    // ── threat aggregation ────────────────────────────────────────

    #[test]
    fn aggregate_threats_returns_default_envelope_when_none_present() {
        let out = aggregate_threats(&[(None, Some(1)), (None, None)]);
        assert_eq!(out.level, "info");
        assert_eq!(out.source, "keyword");
        assert_eq!(out.category.as_deref(), Some("general"));
        assert_eq!(out.confidence, Some(0.3));
    }

    #[test]
    fn aggregate_threats_picks_max_level_via_priority() {
        let high = ThreatClassification {
            level: "high".into(),
            source: "bart".into(),
            category: Some("military".into()),
            confidence: Some(0.8),
        };
        let medium = ThreatClassification {
            level: "medium".into(),
            source: "keyword".into(),
            category: Some("military".into()),
            confidence: Some(0.6),
        };
        let critical = ThreatClassification {
            level: "critical".into(),
            source: "bart".into(),
            category: Some("disaster".into()),
            confidence: Some(0.95),
        };
        let out = aggregate_threats(&[
            (Some(&medium), Some(2)),
            (Some(&high), Some(1)),
            (Some(&critical), Some(3)),
        ]);
        assert_eq!(out.level, "critical");
    }

    #[test]
    fn aggregate_threats_picks_modal_category() {
        let mil = ThreatClassification {
            level: "high".into(),
            source: "bart".into(),
            category: Some("military".into()),
            confidence: Some(0.5),
        };
        let dis = ThreatClassification {
            level: "high".into(),
            source: "bart".into(),
            category: Some("disaster".into()),
            confidence: Some(0.5),
        };
        // 2 military, 1 disaster → military wins.
        let out = aggregate_threats(&[
            (Some(&mil), Some(2)),
            (Some(&mil), Some(2)),
            (Some(&dis), Some(2)),
        ]);
        assert_eq!(out.category.as_deref(), Some("military"));
    }

    #[test]
    fn aggregate_threats_weights_confidence_by_tier() {
        let strong_authoritative = ThreatClassification {
            level: "high".into(),
            source: "bart".into(),
            category: Some("military".into()),
            confidence: Some(0.9),
        };
        let weak_blog = ThreatClassification {
            level: "high".into(),
            source: "bart".into(),
            category: Some("military".into()),
            confidence: Some(0.1),
        };
        // tier 1 (weight 5) × 0.9 + tier 5 (weight 1) × 0.1 = 4.6,
        // total weight 6 → 0.7666… The naive average would be 0.5.
        let out = aggregate_threats(&[
            (Some(&strong_authoritative), Some(1)),
            (Some(&weak_blog), Some(5)),
        ]);
        let c = out.confidence.unwrap();
        assert!(c > 0.7 && c < 0.8, "expected ~0.766, got {c}");
    }

    // ── clustering ────────────────────────────────────────────────

    #[test]
    fn cluster_news_core_empty_returns_empty() {
        let out = cluster_news_core::<fn(&str) -> u32>(&[], &(const_resolver as fn(&str) -> u32));
        assert!(out.is_empty());
    }

    #[test]
    fn cluster_news_core_groups_similar_titles() {
        let items = vec![
            item(
                "Iran launches missile strikes on targets in Syria overnight",
                "Reuters",
                Some(1),
                ts(2026, 5, 4, 12),
            ),
            item(
                "Iran launches missile strikes on targets in Syria overnight says officials",
                "AP",
                Some(2),
                ts(2026, 5, 4, 13),
            ),
        ];
        let out = cluster_news_core(&items, &(const_resolver as fn(&str) -> u32));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].source_count, 2);
        // Top sources are tier-sorted; Reuters tier 1 first.
        assert_eq!(out[0].primary_source, "Reuters");
    }

    #[test]
    fn cluster_news_core_separates_unrelated_titles() {
        let items = vec![
            item(
                "Iran launches missile strikes on Syria",
                "Reuters",
                Some(1),
                ts(2026, 5, 4, 12),
            ),
            item(
                "Stock market rallies on tech earnings report",
                "CNBC",
                Some(3),
                ts(2026, 5, 4, 13),
            ),
        ];
        let out = cluster_news_core(&items, &(const_resolver as fn(&str) -> u32));
        assert_eq!(out.len(), 2);
        // Sorted last_updated desc → CNBC's 13:00 is newest.
        assert_eq!(out[0].primary_source, "CNBC");
        assert_eq!(out[1].primary_source, "Reuters");
    }

    #[test]
    fn cluster_news_core_uses_resolver_when_tier_missing() {
        let items = vec![item(
            "A news headline here for the test",
            "MysterySource",
            None,
            ts(2026, 5, 4, 12),
        )];
        // Resolver returns 7 for any source; that becomes the cluster's only top-source tier.
        fn r(_: &str) -> u32 {
            7
        }
        let out = cluster_news_core(&items, &(r as fn(&str) -> u32));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].top_sources[0].tier, 7);
    }

    #[test]
    fn cluster_news_core_picks_primary_by_tier_then_recency() {
        let items = vec![
            // Same title, different tiers: tier 5 newer, tier 2 older.
            item(
                "Iran missile strike kills dozens in Tehran",
                "Blog",
                Some(5),
                ts(2026, 5, 4, 14),
            ),
            item(
                "Iran missile strike kills dozens in Tehran area",
                "Reuters",
                Some(2),
                ts(2026, 5, 4, 12),
            ),
        ];
        let out = cluster_news_core(&items, &(const_resolver as fn(&str) -> u32));
        assert_eq!(out.len(), 1);
        // Tier 2 wins regardless of recency.
        assert_eq!(out[0].primary_source, "Reuters");
    }

    #[test]
    fn cluster_news_core_picks_modal_geo_location_across_members() {
        let mut a = item(
            "Iran missile strike kills dozens in Tehran",
            "Reuters",
            Some(1),
            ts(2026, 5, 4, 12),
        );
        a.lat = Some(35.7);
        a.lon = Some(51.4);
        let mut b = item(
            "Iran missile strike kills dozens in Tehran officials say",
            "AP",
            Some(2),
            ts(2026, 5, 4, 13),
        );
        b.lat = Some(35.7);
        b.lon = Some(51.4);
        let mut c = item(
            "Iran missile strike kills dozens in Tehran live updates",
            "CNN",
            Some(2),
            ts(2026, 5, 4, 14),
        );
        c.lat = Some(34.0); // a different location
        c.lon = Some(50.0);
        let out = cluster_news_core(&[a, b, c], &(const_resolver as fn(&str) -> u32));
        assert_eq!(out.len(), 1);
        // 2 votes for (35.7, 51.4) > 1 vote for (34.0, 50.0).
        assert!((out[0].lat.unwrap() - 35.7).abs() < 1e-9);
        assert!((out[0].lon.unwrap() - 51.4).abs() < 1e-9);
    }

    #[test]
    fn cluster_news_core_threat_aggregates_inside_cluster() {
        let high = ThreatClassification {
            level: "high".into(),
            source: "bart".into(),
            category: Some("military".into()),
            confidence: Some(0.9),
        };
        let mut a = item(
            "Iran missile strike kills dozens in Tehran",
            "Reuters",
            Some(1),
            ts(2026, 5, 4, 12),
        );
        a.threat = Some(high.clone());
        let b = item(
            "Iran missile strike kills dozens in Tehran officials say",
            "AP",
            Some(2),
            ts(2026, 5, 4, 13),
        );
        let out = cluster_news_core(&[a, b], &(const_resolver as fn(&str) -> u32));
        assert_eq!(out.len(), 1);
        let t = out[0].threat.as_ref().unwrap();
        assert_eq!(t.level, "high");
        assert_eq!(t.category.as_deref(), Some("military"));
    }

    #[test]
    fn cluster_news_core_first_seen_and_last_updated() {
        let items = vec![
            item(
                "Iran missile strike kills dozens in Tehran",
                "Reuters",
                Some(1),
                ts(2026, 5, 4, 12),
            ),
            item(
                "Iran missile strike kills dozens in Tehran officials say",
                "AP",
                Some(2),
                ts(2026, 5, 4, 14),
            ),
        ];
        let out = cluster_news_core(&items, &(const_resolver as fn(&str) -> u32));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].first_seen, ts(2026, 5, 4, 12));
        assert_eq!(out[0].last_updated, ts(2026, 5, 4, 14));
    }

    #[test]
    fn cluster_news_core_id_uses_earliest_pubdate_and_alphanumeric_slug() {
        let items = vec![
            item(
                "Iran's missile-strikes hit, in Syria!",
                "Reuters",
                Some(1),
                ts(2026, 5, 4, 12),
            ),
            item(
                "Iran missile strikes hit Syria officials say",
                "AP",
                Some(2),
                ts(2026, 5, 4, 13),
            ),
        ];
        let out = cluster_news_core(&items, &(const_resolver as fn(&str) -> u32));
        assert_eq!(out.len(), 1);
        let earliest_ms = ts(2026, 5, 4, 12).timestamp_millis();
        // First 20 chars of "Iran's missile-strikes hit, in Syria!" with non-alnum stripped:
        // "I r a n s   m i s s i l e s t r i k e s" → "Iransmissilestrikes" (19 chars,
        // since 20 includes space char which is filtered out — keep first 20 chars
        // then filter, mirroring `slice(0,20).replace(/\W/g,'')`).
        // JS: "Iran's missile-strikes hit, in Syria!".slice(0,20) →
        //     "Iran's missile-strik". replace(/\W/g, '') → "Iransmissilestrik"
        let prefix_20: String = "Iran's missile-strikes hit, in Syria!"
            .chars()
            .take(20)
            .collect();
        let slug: String = prefix_20
            .chars()
            .filter(|c| c.is_ascii_alphanumeric())
            .collect();
        assert_eq!(out[0].id, format!("{earliest_ms}-{slug}"));
    }

    #[test]
    fn cluster_news_core_results_sorted_by_last_updated_desc() {
        let items = vec![
            item(
                "Earlier story about market rally",
                "X",
                Some(2),
                ts(2026, 5, 4, 9),
            ),
            item(
                "Later story about iran strike",
                "Y",
                Some(2),
                ts(2026, 5, 4, 18),
            ),
            item(
                "Mid story unrelated again",
                "Z",
                Some(2),
                ts(2026, 5, 4, 14),
            ),
        ];
        let out = cluster_news_core(&items, &(const_resolver as fn(&str) -> u32));
        assert_eq!(out.len(), 3);
        assert!(out[0].last_updated >= out[1].last_updated);
        assert!(out[1].last_updated >= out[2].last_updated);
    }

    #[test]
    fn cluster_news_core_alert_propagates_when_any_member_alerted() {
        let mut a = item(
            "Iran missile strike",
            "Reuters",
            Some(1),
            ts(2026, 5, 4, 12),
        );
        a.is_alert = false;
        let mut b = item(
            "Iran missile strike officials say",
            "AP",
            Some(2),
            ts(2026, 5, 4, 13),
        );
        b.is_alert = true;
        let out = cluster_news_core(&[a, b], &(const_resolver as fn(&str) -> u32));
        assert!(out[0].is_alert);
    }

    #[test]
    fn cluster_news_core_top_sources_capped_at_three_and_tier_sorted() {
        let mut items = Vec::new();
        for i in 0..5_u32 {
            items.push(item(
                "Iran missile strike Tehran",
                &format!("Source{i}"),
                Some(i + 1),
                ts(2026, 5, 4, 12 + i),
            ));
        }
        let out = cluster_news_core(&items, &(const_resolver as fn(&str) -> u32));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].top_sources.len(), 3);
        assert!(out[0].top_sources[0].tier <= out[0].top_sources[1].tier);
        assert!(out[0].top_sources[1].tier <= out[0].top_sources[2].tier);
    }

    #[test]
    fn cluster_news_core_lang_inherits_primary() {
        let mut a = item(
            "Iran missile strike Tehran",
            "Reuters",
            Some(1),
            ts(2026, 5, 4, 12),
        );
        a.lang = Some("en".into());
        let mut b = item(
            "Iran missile strike Tehran officials say",
            "Blog",
            Some(5),
            ts(2026, 5, 4, 13),
        );
        b.lang = Some("fa".into());
        let out = cluster_news_core(&[a, b], &(const_resolver as fn(&str) -> u32));
        // Reuters (tier 1) is primary → "en".
        assert_eq!(out[0].lang.as_deref(), Some("en"));
    }
}
