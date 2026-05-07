//! Entity index + extraction — port of
//! `worldmonitor/src/services/entity-index.ts` and
//! `entity-extraction.ts`.
//!
//! The actual entity registry (companies, indices, commodities,
//! cryptos, sectors, countries — ~200 entries in
//! `worldmonitor/src/config/entities.ts`) is product-curated data,
//! not algorithm. This module ports the **algorithm** and exposes
//! a [`EntityIndex`] that callers populate with their own
//! `Vec<EntityEntry>` (the handler / IPC glue passes a registry
//! loaded from JSON, the test path uses an in-memory list).
//!
//! `EntityIndex` also implements [`crate::signals::EntityIndex`]
//! (the small trait used by `detect_market_moves`) so this module
//! is a drop-in replacement for [`crate::signals::NoEntityIndex`]
//! once a registry is available.

use std::collections::{HashMap, HashSet};

use serde::{Deserialize, Serialize};

use crate::news_clustering::ClusteredEvent;

// ============================================================================
// Public types — port of `config/entities.ts:1-11`
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntityType {
    Company,
    Index,
    Commodity,
    Crypto,
    Sector,
    Country,
}

impl EntityType {
    /// Stable lower-snake string used as the `byType` map key.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Company => "company",
            Self::Index => "index",
            Self::Commodity => "commodity",
            Self::Crypto => "crypto",
            Self::Sector => "sector",
            Self::Country => "country",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityEntry {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: EntityType,
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub keywords: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sector: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related: Vec<String>,
}

// ============================================================================
// EntityIndex
// ============================================================================

/// In-memory index over a registry of [`EntityEntry`]. Construct via
/// [`EntityIndex::build`]. Cheap to share via `Arc<EntityIndex>` —
/// none of the lookups mutate the index.
#[derive(Debug, Clone, Default)]
pub struct EntityIndex {
    by_id: HashMap<String, EntityEntry>,
    /// `alias.to_lowercase() → entity.id`. Each alias points to a
    /// single entity (later entries with the same alias overwrite
    /// earlier — same as the JS Map.set semantics).
    by_alias: HashMap<String, String>,
    /// `keyword.to_lowercase() → set of entity ids that declared
    /// it`. One keyword can map to many entities; the JS source
    /// uses `Set<string>`.
    by_keyword: HashMap<String, HashSet<String>>,
    by_sector: HashMap<String, HashSet<String>>,
    by_type: HashMap<EntityType, HashSet<String>>,
}

/// Subset of [`EntityIndex`] fed into the signals orchestrator.
/// Implements [`crate::signals::EntityIndex`] so the `silent_divergence`
/// "searched terms" string surfaces real entity keywords.
impl crate::signals::EntityIndex for EntityIndex {
    fn keywords_for(&self, entity_id: &str) -> Option<Vec<String>> {
        let entry = self.by_id.get(entity_id)?;
        if entry.keywords.is_empty() {
            return None;
        }
        Some(entry.keywords.clone())
    }
}

impl EntityIndex {
    /// Build the index from a slice of registry entries. O(N · M)
    /// where M is the average alias / keyword count per entry.
    /// Callers typically build once at boot and share the result.
    #[must_use]
    pub fn build(entries: &[EntityEntry]) -> Self {
        let mut by_id = HashMap::with_capacity(entries.len());
        let mut by_alias = HashMap::new();
        let mut by_keyword: HashMap<String, HashSet<String>> = HashMap::new();
        let mut by_sector: HashMap<String, HashSet<String>> = HashMap::new();
        let mut by_type: HashMap<EntityType, HashSet<String>> = HashMap::new();

        for entry in entries {
            by_id.insert(entry.id.clone(), entry.clone());
            for alias in &entry.aliases {
                by_alias.insert(alias.to_lowercase(), entry.id.clone());
            }
            // Self-aliases — id and name (lowercased). Mirrors
            // `entity-index.ts:28-29`.
            by_alias.insert(entry.id.to_lowercase(), entry.id.clone());
            by_alias.insert(entry.name.to_lowercase(), entry.id.clone());

            for keyword in &entry.keywords {
                by_keyword
                    .entry(keyword.to_lowercase())
                    .or_default()
                    .insert(entry.id.clone());
            }
            if let Some(sector) = &entry.sector {
                by_sector
                    .entry(sector.to_lowercase())
                    .or_default()
                    .insert(entry.id.clone());
            }
            by_type
                .entry(entry.kind)
                .or_default()
                .insert(entry.id.clone());
        }

        Self {
            by_id,
            by_alias,
            by_keyword,
            by_sector,
            by_type,
        }
    }

    /// `true` when no entries are loaded. Callers can short-circuit
    /// extraction when this is true.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.by_id.is_empty()
    }

    /// Number of registered entries.
    #[must_use]
    pub fn len(&self) -> usize {
        self.by_id.len()
    }

    /// Lookup by id (exact, case-sensitive).
    #[must_use]
    pub fn get(&self, entity_id: &str) -> Option<&EntityEntry> {
        self.by_id.get(entity_id)
    }

    /// Lookup by alias (case-insensitive). Mirrors
    /// `lookupEntityByAlias` (`entity-index.ts:59-63`).
    #[must_use]
    pub fn lookup_by_alias(&self, alias: &str) -> Option<&EntityEntry> {
        let id = self.by_alias.get(&alias.to_lowercase())?;
        self.by_id.get(id)
    }

    /// All entries declaring `keyword` (case-insensitive). Mirrors
    /// `lookupEntitiesByKeyword`.
    #[must_use]
    pub fn lookup_by_keyword(&self, keyword: &str) -> Vec<&EntityEntry> {
        let Some(ids) = self.by_keyword.get(&keyword.to_lowercase()) else {
            return Vec::new();
        };
        ids.iter().filter_map(|id| self.by_id.get(id)).collect()
    }

    /// All entries in `sector` (case-insensitive). Mirrors
    /// `lookupEntitiesBySector`.
    #[must_use]
    pub fn lookup_by_sector(&self, sector: &str) -> Vec<&EntityEntry> {
        let Some(ids) = self.by_sector.get(&sector.to_lowercase()) else {
            return Vec::new();
        };
        ids.iter().filter_map(|id| self.by_id.get(id)).collect()
    }

    /// Entries explicitly listed under `entry.related`. Mirrors
    /// `findRelatedEntities`.
    #[must_use]
    pub fn related(&self, entity_id: &str) -> Vec<&EntityEntry> {
        let Some(entry) = self.by_id.get(entity_id) else {
            return Vec::new();
        };
        entry
            .related
            .iter()
            .filter_map(|id| self.by_id.get(id))
            .collect()
    }

    /// Display name for `entity_id`. Falls back to the id itself
    /// when no entry is registered. Mirrors
    /// `getEntityDisplayName`.
    #[must_use]
    pub fn display_name(&self, entity_id: &str) -> String {
        self.by_id
            .get(entity_id)
            .map_or_else(|| entity_id.to_string(), |e| e.name.clone())
    }

    /// Find every entity whose alias or keyword appears in `text`,
    /// scored. Mirrors `findEntitiesInText`
    /// (`entity-index.ts:98-144`):
    ///
    /// 1. Iterate aliases (length ≥ 3) — match word-boundary,
    ///    confidence 0.95 if alias > 4 chars else 0.85, type
    ///    `Alias`. First-seen wins per entity.
    /// 2. Iterate keywords (length ≥ 3) — substring match
    ///    (lowercased), confidence 0.7, type `Keyword`. Skip
    ///    entities already added.
    /// 3. Sort by confidence desc, then position asc.
    #[must_use]
    pub fn find_entities_in_text(&self, text: &str) -> Vec<EntityMatch> {
        let mut matches: Vec<EntityMatch> = Vec::new();
        let mut seen: HashSet<String> = HashSet::new();
        let text_lower = text.to_lowercase();

        // Aliases first. Use BTreeMap-style sorted iteration over
        // the alias keys for a deterministic test order even though
        // the JS source iterates in insertion order — both produce
        // the same final set because of the `seen` dedup.
        let mut aliases: Vec<(&String, &String)> = self.by_alias.iter().collect();
        aliases.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));

        for (alias_lower, entity_id) in aliases {
            if alias_lower.chars().count() < 3 {
                continue;
            }
            if seen.contains(entity_id) {
                continue;
            }
            // Word-bounded match. We do this manually rather than
            // building a Regex per alias (the JS source does that
            // too — fine for the registry size).
            if let Some((pos, hit)) = find_word_bounded(&text_lower, alias_lower) {
                let confidence = if alias_lower.chars().count() > 4 {
                    0.95
                } else {
                    0.85
                };
                let matched_text = preserve_case(text, pos, hit.len());
                matches.push(EntityMatch {
                    entity_id: entity_id.clone(),
                    matched_text,
                    match_type: MatchType::Alias,
                    confidence,
                    position: pos,
                });
                seen.insert(entity_id.clone());
            }
        }

        // Keywords next.
        let mut keywords: Vec<(&String, &HashSet<String>)> = self.by_keyword.iter().collect();
        keywords.sort_unstable_by(|(a, _), (b, _)| a.cmp(b));

        for (keyword, entity_ids) in keywords {
            if keyword.chars().count() < 3 {
                continue;
            }
            let Some(pos) = text_lower.find(keyword.as_str()) else {
                continue;
            };
            // Sort entity_ids deterministically within a keyword
            // bucket — JS uses Set iteration which is insertion
            // order; we sort lexically so test output is stable
            // regardless of HashSet insertion order.
            let mut ids: Vec<&String> = entity_ids.iter().collect();
            ids.sort_unstable();
            for entity_id in ids {
                if seen.contains(entity_id) {
                    continue;
                }
                matches.push(EntityMatch {
                    entity_id: entity_id.clone(),
                    matched_text: keyword.clone(),
                    match_type: MatchType::Keyword,
                    confidence: 0.7,
                    position: pos,
                });
                seen.insert(entity_id.clone());
            }
        }

        matches.sort_by(|a, b| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.position.cmp(&b.position))
        });
        matches
    }

    /// Iterate every entry. Useful for diagnostic dumps + the
    /// `getTopEntitiesFromNews` aggregator.
    pub fn iter(&self) -> impl Iterator<Item = &EntityEntry> {
        self.by_id.values()
    }

    /// Count of entries in `entity_type`.
    #[must_use]
    pub fn count_by_type(&self, entity_type: EntityType) -> usize {
        self.by_type.get(&entity_type).map_or(0, HashSet::len)
    }
}

/// Find `needle` (lowercased, alphanumeric-bounded) inside
/// `haystack_lower` (also lowercased). Returns the byte position
/// and the matched text. Mirrors the `\b<alias>\b` regex from the
/// JS source — manual loop avoids per-alias regex compilation cost.
fn find_word_bounded(haystack_lower: &str, needle: &str) -> Option<(usize, String)> {
    let bytes = haystack_lower.as_bytes();
    let needle_bytes = needle.as_bytes();
    if needle_bytes.is_empty() || needle_bytes.len() > bytes.len() {
        return None;
    }
    let mut pos = 0;
    while pos + needle_bytes.len() <= bytes.len() {
        let found_in_remainder = haystack_lower[pos..].find(needle)?;
        let start = pos + found_in_remainder;
        let end = start + needle_bytes.len();
        let before_ok = start == 0 || !is_word_char(bytes[start - 1]);
        let after_ok = end == bytes.len() || !is_word_char(bytes[end]);
        if before_ok && after_ok {
            return Some((start, needle.to_string()));
        }
        pos = start + 1;
    }
    None
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Slice the original (cased) `text` at `[pos..pos+len]` if the
/// boundaries land on UTF-8 char boundaries; otherwise return the
/// lowercased fallback the caller passed in. Used so the JS
/// `match[0]` (which preserves the cased surface form) is
/// reproduced.
fn preserve_case(text: &str, pos: usize, len: usize) -> String {
    let end = pos + len;
    if text.is_char_boundary(pos) && end <= text.len() && text.is_char_boundary(end) {
        text[pos..end].to_string()
    } else {
        // Boundary mismatch (rare — would require a multi-byte
        // char inside the matched range that wasn't ascii-aligned).
        // Fall back to the lowercased portion.
        text.chars()
            .skip(text[..pos.min(text.len())].chars().count())
            .take(len)
            .collect()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchType {
    Alias,
    Keyword,
    Name,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityMatch {
    #[serde(rename = "entityId")]
    pub entity_id: String,
    #[serde(rename = "matchedText")]
    pub matched_text: String,
    #[serde(rename = "matchType")]
    pub match_type: MatchType,
    pub confidence: f64,
    pub position: usize,
}

// ============================================================================
// Extraction — port of `entity-extraction.ts`
// ============================================================================

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExtractedEntity {
    #[serde(rename = "entityId")]
    pub entity_id: String,
    pub name: String,
    #[serde(rename = "matchedText")]
    pub matched_text: String,
    #[serde(rename = "matchType")]
    pub match_type: MatchType,
    pub confidence: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NewsEntityContext {
    #[serde(rename = "clusterId")]
    pub cluster_id: String,
    pub title: String,
    pub entities: Vec<ExtractedEntity>,
    #[serde(
        default,
        rename = "primaryEntity",
        skip_serializing_if = "Option::is_none"
    )]
    pub primary_entity: Option<String>,
    #[serde(
        default,
        rename = "relatedEntityIds",
        skip_serializing_if = "Vec::is_empty"
    )]
    pub related_entity_ids: Vec<String>,
}

/// Extract entities from a single news title. Each match in the
/// underlying `find_entities_in_text` becomes an `ExtractedEntity`
/// with the registry-resolved display name. Mirrors
/// `extractEntitiesFromTitle`.
#[must_use]
pub fn extract_entities_from_title(index: &EntityIndex, title: &str) -> Vec<ExtractedEntity> {
    index
        .find_entities_in_text(title)
        .into_iter()
        .map(|m| ExtractedEntity {
            name: index.display_name(&m.entity_id),
            entity_id: m.entity_id,
            matched_text: m.matched_text,
            match_type: m.match_type,
            confidence: m.confidence,
        })
        .collect()
}

/// Extract entities for a [`ClusteredEvent`]: reads the primary
/// title PLUS the first 5 cluster items' titles. Items beyond the
/// primary that introduce a new entity get a 0.9× confidence
/// penalty (matches `extractEntitiesFromCluster:52`).
#[must_use]
pub fn extract_entities_from_cluster(
    index: &EntityIndex,
    cluster: &ClusteredEvent,
) -> NewsEntityContext {
    let mut entity_map: HashMap<String, ExtractedEntity> = HashMap::new();
    let mut order: Vec<String> = Vec::new();

    for e in extract_entities_from_title(index, &cluster.primary_title) {
        if !entity_map.contains_key(&e.entity_id) {
            order.push(e.entity_id.clone());
            entity_map.insert(e.entity_id.clone(), e);
        }
    }

    if cluster.all_items.len() > 1 {
        for item in cluster.all_items.iter().take(5) {
            for mut e in extract_entities_from_title(index, &item.title) {
                if !entity_map.contains_key(&e.entity_id) {
                    e.confidence *= 0.9;
                    order.push(e.entity_id.clone());
                    entity_map.insert(e.entity_id.clone(), e);
                }
            }
        }
    }

    let mut entities: Vec<ExtractedEntity> = order
        .iter()
        .filter_map(|id| entity_map.remove(id))
        .collect();
    entities.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let primary_entity = entities.first().map(|e| e.entity_id.clone());

    let mut related_ids: HashSet<String> = HashSet::new();
    for entity in &entities {
        for rel in index.related(&entity.entity_id) {
            related_ids.insert(rel.id.clone());
        }
    }
    let mut related_entity_ids: Vec<String> = related_ids.into_iter().collect();
    related_entity_ids.sort();

    NewsEntityContext {
        cluster_id: cluster.id.clone(),
        title: cluster.primary_title.clone(),
        entities,
        primary_entity,
        related_entity_ids,
    }
}

/// Build the `cluster_id → context` map for a list of clusters.
/// Mirrors `extractEntitiesFromClusters`.
#[must_use]
pub fn extract_entities_from_clusters(
    index: &EntityIndex,
    clusters: &[ClusteredEvent],
) -> HashMap<String, NewsEntityContext> {
    let mut out = HashMap::with_capacity(clusters.len());
    for c in clusters {
        out.insert(c.id.clone(), extract_entities_from_cluster(index, c));
    }
    out
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EntityNewsMatch {
    #[serde(rename = "clusterId")]
    pub cluster_id: String,
    pub title: String,
    pub confidence: f64,
}

/// Find clusters mentioning `entity_id`. Direct mentions keep the
/// extraction confidence; mentions of an entity in `entity.related`
/// get a 0.8× penalty. Mirrors `findNewsForEntity`.
#[must_use]
pub fn find_news_for_entity(
    index: &EntityIndex,
    entity_id: &str,
    contexts: &HashMap<String, NewsEntityContext>,
) -> Vec<EntityNewsMatch> {
    let Some(entry) = index.get(entity_id) else {
        return Vec::new();
    };
    let mut related_ids: HashSet<String> = HashSet::new();
    related_ids.insert(entity_id.to_string());
    for rel in &entry.related {
        related_ids.insert(rel.clone());
    }

    let mut matches: Vec<EntityNewsMatch> = Vec::new();
    for (cluster_id, ctx) in contexts {
        if let Some(direct) = ctx.entities.iter().find(|e| e.entity_id == entity_id) {
            matches.push(EntityNewsMatch {
                cluster_id: cluster_id.clone(),
                title: ctx.title.clone(),
                confidence: direct.confidence,
            });
            continue;
        }
        if let Some(rel) = ctx
            .entities
            .iter()
            .find(|e| related_ids.contains(&e.entity_id))
        {
            matches.push(EntityNewsMatch {
                cluster_id: cluster_id.clone(),
                title: ctx.title.clone(),
                confidence: rel.confidence * 0.8,
            });
        }
    }
    matches.sort_by(|a, b| {
        b.confidence
            .partial_cmp(&a.confidence)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    matches
}

/// Convenience alias matching the JS `findNewsForMarketSymbol`.
#[must_use]
pub fn find_news_for_market_symbol(
    index: &EntityIndex,
    symbol: &str,
    contexts: &HashMap<String, NewsEntityContext>,
) -> Vec<EntityNewsMatch> {
    find_news_for_entity(index, symbol, contexts)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TopEntity {
    #[serde(rename = "entityId")]
    pub entity_id: String,
    pub name: String,
    #[serde(rename = "mentionCount")]
    pub mention_count: usize,
    #[serde(rename = "avgConfidence")]
    pub avg_confidence: f64,
}

/// Top-N entities by mention count across all `contexts`. Average
/// confidence is the mean of every appearance's confidence. Mirrors
/// `getTopEntitiesFromNews` (default `limit = 10`).
#[must_use]
pub fn get_top_entities_from_news(
    index: &EntityIndex,
    contexts: &HashMap<String, NewsEntityContext>,
    limit: usize,
) -> Vec<TopEntity> {
    let mut stats: HashMap<String, (usize, f64)> = HashMap::new();
    for ctx in contexts.values() {
        for e in &ctx.entities {
            let s = stats.entry(e.entity_id.clone()).or_insert((0, 0.0));
            s.0 += 1;
            s.1 += e.confidence;
        }
    }
    let mut top: Vec<TopEntity> = stats
        .into_iter()
        .map(|(entity_id, (count, total))| {
            let name = index.display_name(&entity_id);
            let avg = if count > 0 { total / count as f64 } else { 0.0 };
            TopEntity {
                entity_id,
                name,
                mention_count: count,
                avg_confidence: avg,
            }
        })
        .collect();
    top.sort_by(|a, b| {
        b.mention_count
            .cmp(&a.mention_count)
            .then_with(|| {
                b.avg_confidence
                    .partial_cmp(&a.avg_confidence)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            // Stable lexical tie-break for determinism.
            .then_with(|| a.entity_id.cmp(&b.entity_id))
    });
    top.truncate(limit);
    top
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::news_clustering::{ClusteredEvent, NewsItemCore};
    use chrono::{TimeZone, Utc};

    fn ts(h: u32) -> chrono::DateTime<chrono::Utc> {
        Utc.with_ymd_and_hms(2026, 5, 4, h, 0, 0).unwrap()
    }

    fn entity(
        id: &str,
        kind: EntityType,
        name: &str,
        aliases: &[&str],
        keywords: &[&str],
    ) -> EntityEntry {
        EntityEntry {
            id: id.into(),
            kind,
            name: name.into(),
            aliases: aliases.iter().map(|s| (*s).to_string()).collect(),
            keywords: keywords.iter().map(|s| (*s).to_string()).collect(),
            sector: None,
            related: Vec::new(),
        }
    }

    fn registry() -> Vec<EntityEntry> {
        vec![
            EntityEntry {
                id: "TSLA".into(),
                kind: EntityType::Company,
                name: "Tesla, Inc.".into(),
                aliases: vec!["Tesla".into(), "TSLA".into()],
                keywords: vec!["electric vehicle".into(), "ev".into()],
                sector: Some("Consumer Cyclical".into()),
                related: vec!["AAPL".into()],
            },
            EntityEntry {
                id: "AAPL".into(),
                kind: EntityType::Company,
                name: "Apple Inc.".into(),
                aliases: vec!["Apple".into(), "AAPL".into()],
                keywords: vec!["iphone".into(), "macbook".into()],
                sector: Some("Technology".into()),
                related: vec!["TSLA".into()],
            },
            entity(
                "CL=F",
                EntityType::Commodity,
                "WTI Crude Oil",
                &["WTI", "crude oil"],
                &["oil", "crude"],
            ),
            entity("UA", EntityType::Country, "Ukraine", &["Ukraine"], &[]),
        ]
    }

    fn news_item(title: &str, source: &str, when_h: u32) -> NewsItemCore {
        NewsItemCore {
            source: source.into(),
            title: title.into(),
            link: format!("http://x/{source}"),
            pub_date: ts(when_h),
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

    fn cluster(id: &str, primary: &str, items: Vec<NewsItemCore>) -> ClusteredEvent {
        let last = items.iter().map(|i| i.pub_date).max().unwrap();
        let first = items.iter().map(|i| i.pub_date).min().unwrap();
        ClusteredEvent {
            id: id.into(),
            primary_title: primary.into(),
            primary_source: items[0].source.clone(),
            primary_link: items[0].link.clone(),
            source_count: items.len(),
            top_sources: Vec::new(),
            all_items: items,
            first_seen: first,
            last_updated: last,
            is_alert: false,
            monitor_color: None,
            velocity: None,
            threat: None,
            lat: None,
            lon: None,
            lang: None,
        }
    }

    // ── EntityIndex construction ──────────────────────────────

    #[test]
    fn build_indexes_aliases_keywords_sectors_types() {
        let idx = EntityIndex::build(&registry());
        assert_eq!(idx.len(), 4);
        assert!(!idx.is_empty());
        assert_eq!(idx.count_by_type(EntityType::Company), 2);
        assert_eq!(idx.count_by_type(EntityType::Commodity), 1);
        assert_eq!(idx.count_by_type(EntityType::Country), 1);
        // Self-aliases (id + name) registered.
        assert_eq!(
            idx.lookup_by_alias("tesla").map(|e| e.id.as_str()),
            Some("TSLA")
        );
        assert_eq!(
            idx.lookup_by_alias("Tesla, Inc.").map(|e| e.id.as_str()),
            Some("TSLA")
        );
    }

    #[test]
    fn empty_index_returns_empty_for_every_lookup() {
        let idx = EntityIndex::default();
        assert!(idx.is_empty());
        assert_eq!(idx.len(), 0);
        assert!(idx.get("TSLA").is_none());
        assert!(idx.lookup_by_alias("Tesla").is_none());
        assert!(idx.lookup_by_keyword("oil").is_empty());
        assert!(idx.related("TSLA").is_empty());
        assert!(idx.find_entities_in_text("Tesla earnings").is_empty());
    }

    // ── lookups ───────────────────────────────────────────────

    #[test]
    fn lookup_by_keyword_returns_every_entry_that_declared_it() {
        let idx = EntityIndex::build(&registry());
        let oil_entries = idx.lookup_by_keyword("oil");
        assert_eq!(oil_entries.len(), 1);
        assert_eq!(oil_entries[0].id, "CL=F");
        // Case-insensitive lookup.
        assert_eq!(idx.lookup_by_keyword("OIL").len(), 1);
    }

    #[test]
    fn related_resolves_referenced_entity_ids() {
        let idx = EntityIndex::build(&registry());
        let related = idx.related("TSLA");
        assert_eq!(related.len(), 1);
        assert_eq!(related[0].id, "AAPL");
    }

    #[test]
    fn related_dropped_when_id_missing_from_registry() {
        let mut entries = registry();
        entries[0].related = vec!["NOT_REGISTERED".into()];
        let idx = EntityIndex::build(&entries);
        assert!(idx.related("TSLA").is_empty());
    }

    #[test]
    fn display_name_falls_back_to_id_when_missing() {
        let idx = EntityIndex::build(&registry());
        assert_eq!(idx.display_name("TSLA"), "Tesla, Inc.");
        assert_eq!(idx.display_name("NOT_REGISTERED"), "NOT_REGISTERED");
    }

    // ── find_entities_in_text ────────────────────────────────

    #[test]
    fn find_entities_alias_match_word_bounded() {
        let idx = EntityIndex::build(&registry());
        let m = idx.find_entities_in_text("Tesla beats earnings expectations");
        // Tesla alias matches.
        assert!(m.iter().any(|x| x.entity_id == "TSLA"));
    }

    #[test]
    fn find_entities_no_match_inside_other_word() {
        let idx = EntityIndex::build(&registry());
        // "WTI" is a 3-char alias. With word boundaries, "TWTIQ"
        // does NOT match because before-T is alphanumeric.
        let m = idx.find_entities_in_text("ATWTIQ random string");
        assert!(!m.iter().any(|x| x.entity_id == "CL=F"));
    }

    #[test]
    fn find_entities_three_char_alias_matches_at_word_boundary() {
        let idx = EntityIndex::build(&registry());
        let m = idx.find_entities_in_text("WTI futures rally");
        assert!(m.iter().any(|x| x.entity_id == "CL=F"));
    }

    #[test]
    fn find_entities_long_alias_higher_confidence_than_short() {
        // "Tesla" length 5 → confidence 0.95.
        // 3-char "WTI" → confidence 0.85.
        let idx = EntityIndex::build(&registry());
        let m = idx.find_entities_in_text("WTI futures and Tesla earnings");
        let tsla = m
            .iter()
            .find(|x| x.entity_id == "TSLA")
            .expect("expected TSLA match");
        let cl = m
            .iter()
            .find(|x| x.entity_id == "CL=F")
            .expect("expected CL=F match");
        assert!(tsla.confidence > cl.confidence);
    }

    #[test]
    fn find_entities_falls_back_to_keyword_when_alias_misses() {
        // No alias mentions Apple, but the keyword "iphone" does.
        let idx = EntityIndex::build(&registry());
        let m = idx.find_entities_in_text("Latest iphone reviews are positive");
        let apple = m
            .iter()
            .find(|x| x.entity_id == "AAPL")
            .expect("AAPL via keyword");
        assert_eq!(apple.match_type, MatchType::Keyword);
        assert!((apple.confidence - 0.7).abs() < 1e-9);
    }

    #[test]
    fn find_entities_dedupe_per_entity_alias_wins() {
        // Tesla has both alias "Tesla" and keyword "ev". A title
        // mentioning both must still produce one match (the higher-
        // confidence alias wins per the JS `seen` dedup).
        let idx = EntityIndex::build(&registry());
        let m = idx.find_entities_in_text("Tesla unveils new ev model");
        let count = m.iter().filter(|x| x.entity_id == "TSLA").count();
        assert_eq!(count, 1);
        assert_eq!(
            m.iter().find(|x| x.entity_id == "TSLA").unwrap().match_type,
            MatchType::Alias
        );
    }

    #[test]
    fn find_entities_short_keyword_under_3_chars_skipped() {
        let entries = vec![entity(
            "X",
            EntityType::Company,
            "Twitter",
            &["X"], // 1-char alias < 3 → skipped
            &["x"], // 1-char keyword < 3 → skipped
        )];
        let idx = EntityIndex::build(&entries);
        // Even though "X" appears in the text, neither alias nor
        // keyword path emits a match.
        let m = idx.find_entities_in_text("Stock X rallied");
        assert!(m.is_empty(), "expected no match for sub-3-char tokens");
    }

    // ── extraction over clusters ─────────────────────────────

    #[test]
    fn extract_entities_from_cluster_walks_first_5_items_with_penalty() {
        let idx = EntityIndex::build(&registry());
        let items = vec![
            news_item("Tesla earnings", "Reuters", 12),
            news_item("WTI crude oil futures rally", "Bloomberg", 13),
            news_item("Latest macbook reviews positive", "TechCrunch", 14),
            news_item("Filler one", "Source4", 15),
            news_item("Filler two", "Source5", 16),
            news_item("Filler three (sixth — should NOT be walked)", "Source6", 17),
        ];
        let c = cluster("c1", "Tesla earnings", items);
        let ctx = extract_entities_from_cluster(&idx, &c);

        // TSLA from the primary title — confidence 0.95 (no penalty).
        let tsla = ctx
            .entities
            .iter()
            .find(|e| e.entity_id == "TSLA")
            .expect("TSLA in cluster");
        assert!((tsla.confidence - 0.95).abs() < 1e-9);

        // CL=F from item 1 ("WTI crude oil futures rally"). The
        // alias-iteration phase tries every registered alias for
        // each entity; CL=F has BOTH "WTI" (3 chars → 0.85) and
        // "crude oil" (9 chars > 4 → 0.95). The longer alias is
        // hit first by find_entities_in_text and `seen` dedup
        // keeps it. Item walk applies the 0.9 cluster penalty:
        // 0.95 × 0.9 = 0.855.
        let cl = ctx
            .entities
            .iter()
            .find(|e| e.entity_id == "CL=F")
            .expect("CL=F in cluster");
        assert!(
            (cl.confidence - 0.95 * 0.9).abs() < 1e-9,
            "expected CL=F confidence 0.855, got {}",
            cl.confidence
        );

        // AAPL from item 2 via keyword "macbook" (0.7), penalised
        // by 0.9 → 0.63.
        let aapl = ctx
            .entities
            .iter()
            .find(|e| e.entity_id == "AAPL")
            .expect("AAPL via keyword");
        assert!((aapl.confidence - 0.7 * 0.9).abs() < 1e-9);

        // Sorted by confidence descending.
        let confidences: Vec<f64> = ctx.entities.iter().map(|e| e.confidence).collect();
        for w in confidences.windows(2) {
            assert!(w[0] >= w[1], "entities must be sorted by confidence desc");
        }

        assert_eq!(ctx.primary_entity.as_deref(), Some("TSLA"));
        // Related: TSLA → AAPL, AAPL → TSLA → both in the related set.
        // (AAPL is also in the entities list, but the `related` set
        // is independent — it captures the registry's `related` field.)
        assert!(ctx.related_entity_ids.contains(&"AAPL".to_string()));
        assert!(ctx.related_entity_ids.contains(&"TSLA".to_string()));
    }

    #[test]
    fn extract_entities_from_cluster_single_item_skips_walk() {
        let idx = EntityIndex::build(&registry());
        let c = cluster(
            "c2",
            "Tesla earnings",
            vec![news_item("Tesla earnings", "Reuters", 12)],
        );
        let ctx = extract_entities_from_cluster(&idx, &c);
        // Only the primary title is walked because `all_items.len() == 1`.
        assert_eq!(ctx.entities.len(), 1);
        assert_eq!(ctx.entities[0].entity_id, "TSLA");
    }

    // ── find_news_for_entity ─────────────────────────────────

    #[test]
    fn find_news_for_entity_direct_then_related() {
        let idx = EntityIndex::build(&registry());
        let mut contexts: HashMap<String, NewsEntityContext> = HashMap::new();
        contexts.insert(
            "c1".into(),
            NewsEntityContext {
                cluster_id: "c1".into(),
                title: "Tesla earnings beat".into(),
                entities: vec![ExtractedEntity {
                    entity_id: "TSLA".into(),
                    name: "Tesla, Inc.".into(),
                    matched_text: "Tesla".into(),
                    match_type: MatchType::Alias,
                    confidence: 0.95,
                }],
                primary_entity: Some("TSLA".into()),
                related_entity_ids: vec![],
            },
        );
        contexts.insert(
            "c2".into(),
            NewsEntityContext {
                cluster_id: "c2".into(),
                title: "Apple earnings beat".into(),
                entities: vec![ExtractedEntity {
                    entity_id: "AAPL".into(),
                    name: "Apple Inc.".into(),
                    matched_text: "Apple".into(),
                    match_type: MatchType::Alias,
                    confidence: 0.95,
                }],
                primary_entity: Some("AAPL".into()),
                related_entity_ids: vec![],
            },
        );

        let m = find_news_for_entity(&idx, "TSLA", &contexts);
        // c1 direct (0.95), c2 related (AAPL is TSLA's related → 0.95 * 0.8 = 0.76).
        assert_eq!(m.len(), 2);
        assert_eq!(m[0].cluster_id, "c1");
        assert!((m[0].confidence - 0.95).abs() < 1e-9);
        assert_eq!(m[1].cluster_id, "c2");
        assert!((m[1].confidence - 0.95 * 0.8).abs() < 1e-9);
    }

    #[test]
    fn find_news_for_unknown_entity_returns_empty() {
        let idx = EntityIndex::build(&registry());
        let contexts = HashMap::new();
        let m = find_news_for_entity(&idx, "NOT_REGISTERED", &contexts);
        assert!(m.is_empty());
    }

    #[test]
    fn find_news_for_market_symbol_is_alias_for_find_news_for_entity() {
        let idx = EntityIndex::build(&registry());
        let mut contexts: HashMap<String, NewsEntityContext> = HashMap::new();
        contexts.insert(
            "c1".into(),
            NewsEntityContext {
                cluster_id: "c1".into(),
                title: "WTI rally".into(),
                entities: vec![ExtractedEntity {
                    entity_id: "CL=F".into(),
                    name: "WTI Crude Oil".into(),
                    matched_text: "WTI".into(),
                    match_type: MatchType::Alias,
                    confidence: 0.85,
                }],
                primary_entity: Some("CL=F".into()),
                related_entity_ids: vec![],
            },
        );
        let m = find_news_for_market_symbol(&idx, "CL=F", &contexts);
        assert_eq!(m.len(), 1);
        assert_eq!(m[0].cluster_id, "c1");
    }

    // ── get_top_entities_from_news ───────────────────────────

    #[test]
    fn get_top_entities_orders_by_mention_count_then_confidence() {
        let idx = EntityIndex::build(&registry());
        let mut contexts: HashMap<String, NewsEntityContext> = HashMap::new();
        let mut add = |cluster_id: &str, ents: Vec<(&str, f64)>| {
            contexts.insert(
                cluster_id.into(),
                NewsEntityContext {
                    cluster_id: cluster_id.into(),
                    title: "t".into(),
                    entities: ents
                        .into_iter()
                        .map(|(id, conf)| ExtractedEntity {
                            entity_id: id.into(),
                            name: idx.display_name(id),
                            matched_text: id.into(),
                            match_type: MatchType::Alias,
                            confidence: conf,
                        })
                        .collect(),
                    primary_entity: None,
                    related_entity_ids: vec![],
                },
            );
        };
        add("c1", vec![("TSLA", 0.95), ("AAPL", 0.95)]);
        add("c2", vec![("TSLA", 0.9)]);
        add("c3", vec![("CL=F", 0.85)]);

        let top = get_top_entities_from_news(&idx, &contexts, 10);
        // TSLA: 2 mentions, avg 0.925
        // AAPL: 1, 0.95
        // CL=F: 1, 0.85
        // Order: TSLA (2 mentions wins), then by avg conf among
        // 1-mention rows: AAPL (0.95) > CL=F (0.85).
        assert_eq!(top.len(), 3);
        assert_eq!(top[0].entity_id, "TSLA");
        assert_eq!(top[0].mention_count, 2);
        assert_eq!(top[1].entity_id, "AAPL");
        assert_eq!(top[2].entity_id, "CL=F");
    }

    #[test]
    fn get_top_entities_truncates_to_limit() {
        let idx = EntityIndex::build(&registry());
        let mut contexts: HashMap<String, NewsEntityContext> = HashMap::new();
        contexts.insert(
            "c1".into(),
            NewsEntityContext {
                cluster_id: "c1".into(),
                title: "t".into(),
                entities: vec![
                    ExtractedEntity {
                        entity_id: "TSLA".into(),
                        name: "Tesla, Inc.".into(),
                        matched_text: "Tesla".into(),
                        match_type: MatchType::Alias,
                        confidence: 0.9,
                    },
                    ExtractedEntity {
                        entity_id: "AAPL".into(),
                        name: "Apple Inc.".into(),
                        matched_text: "Apple".into(),
                        match_type: MatchType::Alias,
                        confidence: 0.9,
                    },
                ],
                primary_entity: None,
                related_entity_ids: vec![],
            },
        );
        let top = get_top_entities_from_news(&idx, &contexts, 1);
        assert_eq!(top.len(), 1);
    }

    // ── EntityIndex impls signals::EntityIndex ───────────────

    #[test]
    fn signals_entity_index_trait_returns_keywords() {
        use crate::signals::EntityIndex as SignalsEntityIndex;
        let idx = EntityIndex::build(&registry());
        let kws = SignalsEntityIndex::keywords_for(&idx, "TSLA").expect("TSLA has keywords");
        assert!(kws.iter().any(|k| k == "ev"));
    }

    #[test]
    fn signals_entity_index_trait_returns_none_for_unknown() {
        use crate::signals::EntityIndex as SignalsEntityIndex;
        let idx = EntityIndex::build(&registry());
        assert!(SignalsEntityIndex::keywords_for(&idx, "NOT_REGISTERED").is_none());
    }
}
