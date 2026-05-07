//! Shared types for the ML engine surface.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Errors emitted by every [`crate::MlEngine`] implementation.
#[derive(Debug, Error)]
pub enum MlError {
    /// Underlying HTTP transport failed (network, TLS, DNS).
    #[error("http: {0}")]
    Http(#[from] reqwest::Error),

    /// Upstream returned a non-2xx status with the captured body.
    /// Held inline (not via `from`) because we want the body text in
    /// the error message for diagnosis.
    #[error("upstream {status} from {endpoint}: {body}")]
    Upstream {
        endpoint: &'static str,
        status: u16,
        body: String,
    },

    /// Response body could not be parsed as the expected JSON shape.
    /// `body` carries the (truncated) raw response so the operator
    /// can see what the upstream actually returned.
    #[error("decode error from {endpoint}: {message}: {body}")]
    Decode {
        endpoint: &'static str,
        message: String,
        body: String,
    },

    /// Required configuration (API key, base URL) was empty.
    /// `pellucid-edge-bin` should fail closed and surface a 503 to
    /// callers when this is returned at construction.
    #[error("missing config: {0}")]
    MissingConfig(&'static str),

    /// Backend doesn't implement this method (e.g. an embeddings-only
    /// engine being asked to summarize). Callers should compose
    /// engines via [`crate::CompositeEngine`] to avoid this at
    /// runtime; the variant exists so trait conformance stays clean
    /// for partial backends.
    #[error("operation `{0}` not supported by this engine")]
    Unsupported(&'static str),

    /// Caller passed empty input where a non-empty string was
    /// required.
    #[error("empty input not allowed for `{0}`")]
    EmptyInput(&'static str),

    /// Upstream returned a structured response we could parse but it
    /// violates an invariant (wrong vector dimension, missing
    /// required field, etc.). Distinct from [`MlError::Decode`]
    /// which means the bytes weren't valid JSON for the schema.
    #[error("invalid response from {endpoint}: {message}")]
    InvalidResponse {
        endpoint: &'static str,
        message: String,
    },
}

/// Sentiment label returned by [`crate::MlEngine::sentiment`].
///
/// Three-way classification matches the Groq prompt template the
/// `GroqEngine` uses; the original WorldMonitor sentiment task
/// (`Xenova/distilbert-sst2`) was binary positive/negative — the
/// `Neutral` arm is a strict superset that lets the prompt return
/// "none of the above" instead of forcing a low-confidence guess.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SentimentLabel {
    Positive,
    Negative,
    Neutral,
}

impl SentimentLabel {
    /// Parse a label from the lowercased Groq response. Returns
    /// `None` for any unknown token so the caller can decide whether
    /// to treat it as `Neutral` or surface an error.
    #[must_use]
    pub fn parse(raw: &str) -> Option<Self> {
        match raw.trim().to_lowercase().as_str() {
            "positive" | "pos" | "+" => Some(Self::Positive),
            "negative" | "neg" | "-" => Some(Self::Negative),
            "neutral" | "none" | "mixed" => Some(Self::Neutral),
            _ => None,
        }
    }

    /// Stable string form used in cache keys + JSON payloads.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Positive => "positive",
            Self::Negative => "negative",
            Self::Neutral => "neutral",
        }
    }
}

/// Sentiment result with confidence in `[0, 1]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Sentiment {
    pub label: SentimentLabel,
    /// Confidence the label is correct, in `[0, 1]`. For Groq this
    /// is derived from the model's self-reported confidence in JSON
    /// mode; for any backend that can't return one we synthesise
    /// `0.5` (neutral) and document the source on the error.
    pub confidence: f64,
}

/// Named entity result from [`crate::MlEngine::extract_entities`].
///
/// `kind` mirrors the OntoNotes / spaCy NER tagset that the Groq
/// extraction prompt requests — common values include `"PERSON"`,
/// `"ORG"`, `"LOC"` (location), `"GPE"` (geo-political entity),
/// `"EVENT"`, `"PRODUCT"`, `"DATE"`, `"MONEY"`. Stored as a `String`
/// (rather than an `enum`) so model-emitted novel categories are
/// preserved for downstream filtering.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Entity {
    /// Surface form as it appeared in the source text.
    pub text: String,
    /// Entity category — see module docs for typical values.
    pub kind: String,
    /// Self-reported confidence in `[0, 1]`. Optional because some
    /// backends (HF NER) emit it and some (Groq prompts) only do so
    /// when explicitly asked for in JSON mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f64>,
    /// Inclusive start byte offset of `text` within the original
    /// input, when the backend provides it. `None` for LLM-extracted
    /// entities since chat prompts don't preserve offsets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub start: Option<usize>,
    /// Exclusive end byte offset (same caveat as `start`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub end: Option<usize>,
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn sentiment_label_parses_canonical_forms() {
        assert_eq!(SentimentLabel::parse("positive"), Some(SentimentLabel::Positive));
        assert_eq!(SentimentLabel::parse("Negative"), Some(SentimentLabel::Negative));
        assert_eq!(SentimentLabel::parse("  NEUTRAL  "), Some(SentimentLabel::Neutral));
    }

    #[test]
    fn sentiment_label_parses_short_aliases() {
        assert_eq!(SentimentLabel::parse("pos"), Some(SentimentLabel::Positive));
        assert_eq!(SentimentLabel::parse("neg"), Some(SentimentLabel::Negative));
        assert_eq!(SentimentLabel::parse("+"), Some(SentimentLabel::Positive));
        assert_eq!(SentimentLabel::parse("-"), Some(SentimentLabel::Negative));
        assert_eq!(SentimentLabel::parse("none"), Some(SentimentLabel::Neutral));
        assert_eq!(SentimentLabel::parse("mixed"), Some(SentimentLabel::Neutral));
    }

    #[test]
    fn sentiment_label_unknown_returns_none() {
        assert_eq!(SentimentLabel::parse("happy"), None);
        assert_eq!(SentimentLabel::parse(""), None);
    }

    #[test]
    fn sentiment_label_as_str_round_trips_via_parse() {
        for label in [
            SentimentLabel::Positive,
            SentimentLabel::Negative,
            SentimentLabel::Neutral,
        ] {
            assert_eq!(SentimentLabel::parse(label.as_str()), Some(label));
        }
    }

    #[test]
    fn sentiment_label_serializes_lowercase() {
        let s = serde_json::to_string(&SentimentLabel::Positive).unwrap();
        assert_eq!(s, r#""positive""#);
        let parsed: SentimentLabel = serde_json::from_str(r#""negative""#).unwrap();
        assert_eq!(parsed, SentimentLabel::Negative);
    }

    #[test]
    fn entity_round_trips_through_serde_with_optional_offsets() {
        let e = Entity {
            text: "Tehran".into(),
            kind: "GPE".into(),
            confidence: Some(0.92),
            start: Some(10),
            end: Some(16),
        };
        let s = serde_json::to_string(&e).unwrap();
        let back: Entity = serde_json::from_str(&s).unwrap();
        assert_eq!(e, back);
    }

    #[test]
    fn entity_with_none_offsets_omits_fields_in_json() {
        let e = Entity {
            text: "Federal Reserve".into(),
            kind: "ORG".into(),
            confidence: None,
            start: None,
            end: None,
        };
        let s = serde_json::to_string(&e).unwrap();
        // skip_serializing_if = Option::is_none → fields absent
        assert!(!s.contains("confidence"));
        assert!(!s.contains("start"));
        assert!(!s.contains("end"));
    }

    #[test]
    fn ml_error_messages_are_descriptive() {
        let err = MlError::Upstream {
            endpoint: "groq.chat",
            status: 429,
            body: "rate limited".into(),
        };
        let msg = format!("{err}");
        assert!(msg.contains("groq.chat"));
        assert!(msg.contains("429"));
        assert!(msg.contains("rate limited"));
    }
}
