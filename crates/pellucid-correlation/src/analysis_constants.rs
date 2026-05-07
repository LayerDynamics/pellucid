//! Shared constants and pure utility functions for clustering and
//! correlation analysis. Ported 1:1 from
//! `worldmonitor/src/utils/analysis-constants.ts`.
//!
//! Both `clustering` (server-side, port of `scripts/_clustering.mjs`)
//! and the upcoming `engine` module (port of
//! `src/services/analysis-core.ts` + `correlation-engine/`) import
//! `tokenize`, `jaccard_similarity`, the threshold constants, and the
//! keyword sets from this module.

use std::collections::{BTreeMap, HashSet};

use once_cell::sync::Lazy;
use regex::Regex;
use uuid::Uuid;

// ============================================================================
// Clustering constants
// ============================================================================

/// Jaccard similarity threshold above which two news items are
/// considered the same cluster. Matches `SIMILARITY_THRESHOLD = 0.5`
/// in `analysis-constants.ts:11` and `_clustering.mjs:3`.
pub const SIMILARITY_THRESHOLD: f64 = 0.5;

/// English stopwords filtered out by `tokenize`. Mirrors the set in
/// `analysis-constants.ts:13-23` exactly (same words, same order).
pub static STOP_WORDS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        "the", "a", "an", "and", "or", "but", "in", "on", "at", "to", "for", "of", "with", "by",
        "from", "as", "is", "was", "are", "were", "been", "be", "have", "has", "had", "do",
        "does", "did", "will", "would", "could", "should", "may", "might", "must", "shall",
        "can", "need", "it", "its", "this", "that", "these", "those", "i", "you", "he", "she",
        "we", "they", "what", "which", "who", "whom", "how", "when", "where", "why", "all",
        "each", "every", "both", "few", "more", "most", "other", "some", "such", "no", "not",
        "only", "same", "so", "than", "too", "very", "just", "also", "now", "new", "says",
        "said", "after",
    ]
    .into_iter()
    .collect()
});

// ============================================================================
// Correlation thresholds (analysis-constants.ts:25-30)
// ============================================================================

/// Absolute prediction-market shift (in price points) that qualifies a
/// `prediction_leads_news` signal candidate.
pub const PREDICTION_SHIFT_THRESHOLD: f64 = 5.0;

/// Absolute market move (% change) that qualifies a market-explained
/// or silent-divergence signal.
pub const MARKET_MOVE_THRESHOLD: f64 = 2.0;

/// Aggregate news velocity (sources/hr * keyword frequency) that
/// gates `velocity_spike` and reciprocal correlations.
pub const NEWS_VELOCITY_THRESHOLD: f64 = 3.0;

/// Energy commodity move threshold for `flow_price_divergence`.
pub const FLOW_PRICE_THRESHOLD: f64 = 1.5;

/// Symbols treated as energy commodities for divergence detection.
pub static ENERGY_COMMODITY_SYMBOLS: Lazy<HashSet<&'static str>> =
    Lazy::new(|| ["CL=F", "NG=F"].into_iter().collect());

// ============================================================================
// Keyword sets (analysis-constants.ts:32-43)
// ============================================================================

pub static PIPELINE_KEYWORDS: &[&str] = &["pipeline", "pipelines", "line", "terminal"];

pub static FLOW_DROP_KEYWORDS: &[&str] = &[
    "flow",
    "throughput",
    "capacity",
    "outage",
    "leak",
    "rupture",
    "shutdown",
    "maintenance",
    "curtailment",
    "force majeure",
    "halt",
    "halted",
    "reduced",
    "reduction",
    "drop",
    "offline",
    "suspend",
    "suspended",
    "stoppage",
];

pub static TOPIC_KEYWORDS: &[&str] = &[
    "iran",
    "israel",
    "ukraine",
    "russia",
    "china",
    "taiwan",
    "oil",
    "crypto",
    "fed",
    "interest",
    "inflation",
    "recession",
    "war",
    "sanctions",
    "tariff",
    "ai",
    "tech",
    "layoff",
    "trump",
    "biden",
    "election",
];

/// Suppressed trending terms — mirrors `analysis-constants.ts:45-154`
/// exactly. These terms never qualify as `velocity_spike` candidates
/// even if they exceed the absolute threshold.
pub static SUPPRESSED_TRENDING_TERMS: Lazy<HashSet<&'static str>> = Lazy::new(|| {
    [
        // Meta / media terms
        "ai", "app", "api", "new", "top", "big", "ceo", "cto", "update", "report", "latest",
        "breaking", "analysis", "reuters", "exclusive", "opinion", "editorial", "watch",
        "live", "video", "photo", "photos", "read", "full", "source", "sources", "according",
        "ahead", "english", "times", "post", "news", "press", "media", "journal", "morning",
        "evening", "daily", "weekly", "monthly", "newsletter", "subscribe", "podcast",
        "interview",
        // Common news verbs
        "says", "said", "tells", "told", "calls", "called", "makes", "made", "takes", "took",
        "gets", "gives", "gave", "goes", "went", "comes", "came", "puts", "sets", "set",
        "shows", "shown", "finds", "found", "keeps", "kept", "holds", "held", "runs", "turns",
        "turned", "leads", "led", "brings", "brought", "starts", "started", "moves", "moved",
        "plans", "planned", "wants", "wanted", "needs", "needed", "looks", "looked", "works",
        "worked", "tries", "tried", "asks", "asked", "uses", "used", "expects", "expected",
        "reports", "reported", "claims", "claimed", "warns", "warned", "reveals", "revealed",
        "announces", "announced", "confirms", "confirmed", "denies", "denied", "launches",
        "launched", "signs", "signed", "faces", "faced", "seeks", "sought", "hits", "hit",
        "dies", "died", "killed", "kills", "rises", "rose", "falls", "fell", "wins", "won",
        "lost", "ends", "ended", "begins", "began", "opens", "opened", "closes", "closed",
        "raises", "raised", "cuts", "cut", "adds", "added", "drops", "dropped", "pushes",
        "pushed", "pulls", "pulled", "backs", "backed", "blocks", "blocked", "passes",
        "passed", "votes", "voted", "joins", "joined", "leaves", "left", "returns",
        "returned", "sends", "sent", "urges", "urged", "vows", "vowed", "pledges", "pledged",
        "rejects", "rejected", "approves", "approved",
        // Adjectives / adverbs / time words
        "first", "last", "next", "major", "former", "still", "despite", "amid", "over",
        "under", "back", "year", "years", "day", "days", "week", "weeks", "month", "months",
        "time", "long", "high", "low", "part", "early", "late", "key", "two", "three", "four",
        "five", "million", "billion", "percent", "nearly", "almost", "already", "just", "even",
        "since", "while", "during", "before", "between", "again", "against", "into", "through",
        "around", "about", "much", "many", "several", "second", "third", "possible", "likely",
        "least", "best", "worst", "largest", "biggest", "smallest", "highest", "lowest",
        "record", "global", "local",
        // Generic news nouns
        "state", "states", "department", "officials", "official", "country", "countries",
        "people", "group", "groups", "plan", "deal", "talks", "move", "order", "case",
        "house", "court", "secretary", "board", "control", "bank", "power", "leader",
        "leaders", "government", "minister", "president", "agency", "market", "markets",
        "company", "companies", "world", "white", "head", "side", "point", "end", "line",
        "area", "number", "issue", "issues", "policy", "security", "force", "forces",
        "system", "service", "services", "program", "project", "effort", "action", "support",
        "level", "rate", "rates", "price", "prices", "trade", "growth", "change", "changes",
        "crisis", "risk", "impact", "future", "history", "data", "team", "member", "members",
        "office", "sector", "region", "regions", "center", "role", "south", "north", "east",
        "west", "eastern", "western", "southern", "northern", "central", "middle", "united",
        "national", "international", "federal",
        // Base verb forms (NER fallback)
        "say", "get", "give", "go", "come", "put", "take", "make", "know", "think", "see",
        "want", "look", "find", "tell", "ask", "use", "try", "leave", "call", "keep", "let",
        "begin", "show", "hear", "play", "run", "move", "help", "turn", "start", "hold",
        "bring", "write", "provide", "sit", "stand", "lose", "pay", "meet", "include",
        "continue", "learn", "lead", "believe", "feel", "follow", "stop", "speak", "allow",
        "add", "grow", "open", "walk", "win", "offer", "appear", "buy", "wait", "serve",
        "die", "send", "build", "stay", "fall", "reach", "remain", "suggest", "raise", "sell",
        "require", "decide", "develop", "break", "happen", "create", "live",
        // Numbers / misc
        "000", "100", "200", "500", "per", "than",
        // Finance / trading generic
        "trading", "stock", "earnings", "finance", "defi", "ipo", "tradingview", "currency",
        "dollar", "usd", "investing", "equity", "valuation", "ecb", "regulation", "outlook",
        "forecast", "financial",
        // Web / tech generic
        "com", "platform", "block",
        // Generic news nouns (additional)
        "focus", "today", "chief", "basel",
        // Generic adjectives / adverbs (additional)
        "ongoing", "higher", "poised", "track",
        // URL / source fragments
        "wall", "street", "financialcontent", "ray", "msn", "aol",
        // Date fragments
        "2025", "2026", "2027",
        // Months
        "january", "february", "march", "april", "may", "june", "july", "august",
        "september", "october", "november", "december",
        // Company name fragments
        "goldman", "sachs", "off",
        // Basic English stopwords (pronouns / prepositions / adverbs)
        "here", "there", "where", "when", "what", "which", "who", "whom", "this", "that",
        "these", "those", "been", "being", "have", "has", "had", "having", "does", "done",
        "doing", "would", "could", "should", "will", "shall", "might", "must", "also", "more",
        "most", "some", "other", "only", "very", "after", "with", "from", "they", "them",
        "their", "then", "now", "how", "all", "each", "every", "both", "few", "own", "same",
        "such", "too", "any", "well",
    ]
    .into_iter()
    .collect()
});

// ============================================================================
// Topic mappings for `find_related_topics` (analysis-constants.ts:157-168)
// ============================================================================

/// `BTreeMap` for deterministic iteration order so test outputs are
/// stable across runs and platforms.
pub static TOPIC_MAPPINGS: Lazy<BTreeMap<&'static str, &'static [&'static str]>> = Lazy::new(|| {
    let mut m: BTreeMap<&'static str, &'static [&'static str]> = BTreeMap::new();
    m.insert("iran", &["iran", "israel", "oil", "sanctions"]);
    m.insert("israel", &["israel", "iran", "war", "gaza"]);
    m.insert("ukraine", &["ukraine", "russia", "war", "nato"]);
    m.insert("russia", &["russia", "ukraine", "sanctions"]);
    m.insert("china", &["china", "taiwan", "tariff", "trade"]);
    m.insert("taiwan", &["taiwan", "china"]);
    m.insert("trump", &["trump", "election", "tariff"]);
    m.insert("fed", &["fed", "interest", "inflation", "recession"]);
    m.insert("bitcoin", &["crypto", "bitcoin"]);
    m.insert("recession", &["recession", "fed", "inflation"]);
    m
});

// ============================================================================
// Pure utility functions
// ============================================================================

/// Lowercase, strip non-alphanumeric, split on whitespace, filter
/// short/stopword tokens, return as a `HashSet`. Matches
/// `analysis-constants.ts:171-178` and `_clustering.mjs:52-59`.
#[must_use]
pub fn tokenize(text: &str) -> HashSet<String> {
    text.to_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { ' ' })
        .collect::<String>()
        .split_whitespace()
        .filter(|w| w.len() > 2 && !STOP_WORDS.contains(*w))
        .map(String::from)
        .collect()
}

/// Jaccard similarity (|A ∩ B| / |A ∪ B|). Matches
/// `analysis-constants.ts:180-185` — including the `(0,0) -> 0` edge
/// case (the JS impl returns `intersection.size / union.size` which
/// yields `NaN` when both empty; the JS branch returns 0, so we do
/// too).
#[must_use]
pub fn jaccard_similarity(a: &HashSet<String>, b: &HashSet<String>) -> f64 {
    if a.is_empty() && b.is_empty() {
        return 0.0;
    }
    let intersection = a.intersection(b).count();
    // |A ∪ B| = |A| + |B| - |A ∩ B| (avoids allocating a second set,
    // matches the optimised path in `_clustering.mjs:61-69`).
    let union = a.len() + b.len() - intersection;
    intersection as f64 / union as f64
}

/// `true` if any keyword in the slice appears as a substring of
/// `text`. Mirrors `analysis-constants.ts:187-189`.
#[must_use]
pub fn includes_keyword(text: &str, keywords: &[&str]) -> bool {
    keywords.iter().any(|kw| text.contains(kw))
}

/// Word-boundary keyword match. Mirrors
/// `analysis-constants.ts:195-200` (`new RegExp('\\b<kw>\\b', 'i')`).
/// Keyword is lowercased and trimmed; empty keywords return `false`.
#[must_use]
pub fn contains_topic_keyword(text: &str, keyword: &str) -> bool {
    let normalized = keyword.trim().to_lowercase();
    if normalized.is_empty() {
        return false;
    }
    let pattern = format!(r"(?i)\b{}\b", regex::escape(&normalized));
    Regex::new(&pattern)
        .map(|re| re.is_match(text))
        .unwrap_or(false)
}

/// Return all topic groups whose key appears (word-bounded) in
/// `prediction`, deduplicated, preserving first-seen order. Mirrors
/// `analysis-constants.ts:202-213` — the JS uses `Object.entries`
/// (insertion order); we use `BTreeMap` (alphabetical) for
/// determinism, which is a strict superset of the JS behaviour.
#[must_use]
pub fn find_related_topics(prediction: &str) -> Vec<String> {
    let title = prediction.to_lowercase();
    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for (key, topics) in TOPIC_MAPPINGS.iter() {
        if contains_topic_keyword(&title, key) {
            for &t in *topics {
                if seen.insert(t.to_string()) {
                    out.push(t.to_string());
                }
            }
        }
    }
    out
}

/// Generate a fresh signal id of the form `sig-<uuid-v4>`. Mirrors
/// `analysis-constants.ts:215-217`.
#[must_use]
pub fn generate_signal_id() -> String {
    format!("sig-{}", Uuid::new_v4())
}

/// Market-signal types whose dedupe key uses the symbol only
/// (not the rounded numeric value). Mirrors the special-case
/// behaviour at `analysis-constants.ts:222-225`.
const MARKET_SIGNAL_TYPES: &[&str] = &[
    "silent_divergence",
    "flow_price_divergence",
    "explained_market_move",
];

/// Build a deduplication key. For market signals the key is
/// `<type>:<identifier>` (so price wobble doesn't churn). Otherwise
/// the value is rounded to 1 decimal and embedded as
/// `<type>:<identifier>:<rounded>`. Mirrors
/// `analysis-constants.ts:219-228`.
#[must_use]
pub fn generate_dedupe_key(signal_type: &str, identifier: &str, value: f64) -> String {
    if MARKET_SIGNAL_TYPES.contains(&signal_type) {
        return format!("{signal_type}:{identifier}");
    }
    let rounded = (value * 10.0).round() / 10.0;
    // Avoid printing `-0` for tiny negative values that round to zero
    // (JS `Math.round(-0.04 * 10) / 10` yields `0`, not `-0`).
    let rounded = if rounded == 0.0 { 0.0 } else { rounded };
    // JS `${rounded}` prints integers without a trailing `.0`, while
    // Rust's `Display` for `f64` would print `0` as `0` already; for
    // values like `5.5` both produce `5.5`. The behaviour matches.
    format!("{signal_type}:{identifier}:{rounded}")
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn tokenize_drops_stop_words_and_short_tokens() {
        let tokens = tokenize("The quick brown fox jumps over a lazy dog");
        // "the", "a" are stopwords; everything else is length > 2 and unique.
        assert!(tokens.contains("quick"));
        assert!(tokens.contains("brown"));
        assert!(tokens.contains("jumps"));
        assert!(tokens.contains("over"));
        assert!(tokens.contains("lazy"));
        assert!(tokens.contains("dog"));
        assert!(!tokens.contains("the"));
        assert!(!tokens.contains("a"));
        // "fox" is 3 chars: kept (filter is `len > 2`).
        assert!(tokens.contains("fox"));
    }

    #[test]
    fn tokenize_strips_punctuation() {
        let tokens = tokenize("Iran's missile-strikes hit Syria, officials say.");
        // Punctuation collapses to whitespace, so tokens are the words.
        assert!(tokens.contains("iran"));
        assert!(tokens.contains("missile"));
        assert!(tokens.contains("strikes"));
        assert!(tokens.contains("hit"));
        assert!(tokens.contains("syria"));
        assert!(tokens.contains("officials"));
        // "say" is 3 chars and not in STOP_WORDS, so kept.
        assert!(tokens.contains("say"));
    }

    #[test]
    fn jaccard_returns_zero_when_both_sets_empty() {
        let a: HashSet<String> = HashSet::new();
        let b: HashSet<String> = HashSet::new();
        assert_eq!(jaccard_similarity(&a, &b), 0.0);
    }

    #[test]
    fn jaccard_identical_sets_score_one() {
        let s: HashSet<String> = ["iran", "missile", "syria"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        assert!((jaccard_similarity(&s, &s) - 1.0).abs() < 1e-12);
    }

    #[test]
    fn jaccard_disjoint_sets_score_zero() {
        let a: HashSet<String> = ["iran", "missile"].iter().map(|s| (*s).to_string()).collect();
        let b: HashSet<String> = ["stock", "rally"].iter().map(|s| (*s).to_string()).collect();
        assert_eq!(jaccard_similarity(&a, &b), 0.0);
    }

    #[test]
    fn jaccard_partial_overlap_is_intersection_over_union() {
        // |A| = 3, |B| = 3, |A ∩ B| = 2 (iran, missile),
        // |A ∪ B| = 4, similarity = 2/4 = 0.5.
        let a: HashSet<String> = ["iran", "missile", "syria"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        let b: HashSet<String> = ["iran", "missile", "lebanon"]
            .iter()
            .map(|s| (*s).to_string())
            .collect();
        assert!((jaccard_similarity(&a, &b) - 0.5).abs() < 1e-12);
    }

    #[test]
    fn includes_keyword_substring_match() {
        assert!(includes_keyword(
            "Pipeline rupture halts northbound flow",
            PIPELINE_KEYWORDS
        ));
        assert!(includes_keyword(
            "Pipeline rupture halts northbound flow",
            FLOW_DROP_KEYWORDS
        ));
        assert!(!includes_keyword("Stock market rally", PIPELINE_KEYWORDS));
    }

    #[test]
    fn contains_topic_keyword_word_bounded() {
        // `iran` matches; `iranian` should NOT (would only match the
        // substring path of `includes_keyword`).
        assert!(contains_topic_keyword("iran sanctions news", "iran"));
        assert!(!contains_topic_keyword("iranian foreign minister", "iran"));
        assert!(!contains_topic_keyword("", "iran"));
        assert!(!contains_topic_keyword("iran", ""));
    }

    #[test]
    fn find_related_topics_dedupes_across_keys() {
        // "iran in israel" hits both "iran" and "israel" mapping keys;
        // their topic lists overlap on "iran" and "israel" — output
        // must be deduplicated (set semantics).
        let topics = find_related_topics("iran tensions with israel rise");
        let unique: HashSet<String> = topics.iter().cloned().collect();
        assert_eq!(topics.len(), unique.len());
        assert!(topics.contains(&"iran".to_string()));
        assert!(topics.contains(&"israel".to_string()));
    }

    #[test]
    fn find_related_topics_empty_when_no_match() {
        let topics = find_related_topics("nothing relevant here");
        assert!(topics.is_empty());
    }

    #[test]
    fn signal_id_has_sig_prefix_and_uuid_shape() {
        let id = generate_signal_id();
        assert!(id.starts_with("sig-"));
        // sig- (4) + uuid (36) = 40
        assert_eq!(id.len(), 40);
    }

    #[test]
    fn dedupe_key_market_signals_omit_value() {
        // Per analysis-constants.ts:222-225 — these three types use
        // `<type>:<id>` so price wobble doesn't churn dedupe.
        for ty in MARKET_SIGNAL_TYPES {
            let k1 = generate_dedupe_key(ty, "CL=F", 2.4);
            let k2 = generate_dedupe_key(ty, "CL=F", 2.5);
            assert_eq!(k1, k2, "market dedupe must ignore value for {ty}");
            assert_eq!(k1, format!("{ty}:CL=F"));
        }
    }

    #[test]
    fn dedupe_key_non_market_signals_round_to_one_decimal() {
        // 2.44 → 2.4, 2.46 → 2.5 (round-half-away-from-zero).
        assert_eq!(
            generate_dedupe_key("velocity_spike", "iran", 2.44),
            "velocity_spike:iran:2.4"
        );
        assert_eq!(
            generate_dedupe_key("velocity_spike", "iran", 2.46),
            "velocity_spike:iran:2.5"
        );
    }

    #[test]
    fn suppressed_terms_includes_meta_and_news_verbs() {
        // Spot-check coverage of representative entries from each
        // category in analysis-constants.ts:45-154.
        for term in [
            "ai", "breaking", "says", "trading", "january", "south", "ceo",
        ] {
            assert!(
                SUPPRESSED_TRENDING_TERMS.contains(term),
                "missing suppressed term: {term}"
            );
        }
    }

    #[test]
    fn similarity_threshold_matches_source() {
        // Triple-checked against analysis-constants.ts:11 +
        // _clustering.mjs:3 — both define 0.5.
        assert!((SIMILARITY_THRESHOLD - 0.5).abs() < f64::EPSILON);
    }
}
