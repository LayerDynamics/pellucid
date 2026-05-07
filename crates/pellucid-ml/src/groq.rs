//! Groq chat-completions backend.
//!
//! Implements `sentiment`, `summarize`, and `extract_entities` by
//! prompting an instruction-tuned Llama-3.1 (or any Groq-hosted) model
//! at the OpenAI-compatible endpoint:
//!
//! ```text
//! POST https://api.groq.com/openai/v1/chat/completions
//! Authorization: Bearer <GROQ_API_KEY>
//! Content-Type: application/json
//! ```
//!
//! Sentiment + entity extraction request `response_format: json_object`
//! so the model returns a strict JSON envelope; summarize takes plain
//! text. Each request is parameterised with `temperature: 0` to
//! minimise non-determinism (one of Groq's strengths is consistent
//! latency, so deterministic prompting is the natural pairing).
//!
//! `embed` and `batch_embed` are intentionally not supported — Groq
//! does not host an embeddings model. Wire [`crate::HfEmbeddingEngine`]
//! for embeddings and combine them with this engine via
//! [`crate::CompositeEngine`].

use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde::Deserialize;
use serde_json::Value;

use crate::engine::MlEngine;
use crate::types::{Entity, MlError, Sentiment, SentimentLabel};

const DEFAULT_BASE_URL: &str = "https://api.groq.com/openai/v1";

/// Default Groq chat model. Llama-3.1-8B is fast (free tier covers
/// thousands of req/day), follows JSON-mode instructions reliably,
/// and is more than enough for sentiment + entity extraction.
/// Override via [`GroqEngineBuilder::model`] for higher quality
/// (e.g. `llama-3.1-70b-versatile`).
pub const DEFAULT_MODEL: &str = "llama-3.1-8b-instant";

/// Builder for [`GroqEngine`].
#[derive(Debug, Clone)]
pub struct GroqEngineBuilder {
    api_key: String,
    base_url: String,
    model: String,
    client: Option<Client>,
}

impl GroqEngineBuilder {
    /// Construct with an API key.
    #[must_use]
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            model: DEFAULT_MODEL.to_string(),
            client: None,
        }
    }

    /// Override the chat model id.
    #[must_use]
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Override the base URL (used by tests with `wiremock`).
    #[must_use]
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Reuse an existing `reqwest::Client` (shared connection pool).
    #[must_use]
    pub fn http_client(mut self, client: Client) -> Self {
        self.client = Some(client);
        self
    }

    /// Materialise the engine. Empty config returns
    /// [`MlError::MissingConfig`] so the binary fails closed at boot.
    pub fn build(self) -> Result<GroqEngine, MlError> {
        if self.api_key.trim().is_empty() {
            return Err(MlError::MissingConfig("GROQ_API_KEY"));
        }
        if self.model.trim().is_empty() {
            return Err(MlError::MissingConfig("groq_model"));
        }
        if self.base_url.trim().is_empty() {
            return Err(MlError::MissingConfig("groq_base_url"));
        }
        let client = self.client.unwrap_or_default();
        Ok(GroqEngine {
            api_key: self.api_key,
            base_url: self.base_url,
            model: self.model,
            client,
        })
    }
}

/// Groq chat-backed engine.
#[derive(Debug, Clone)]
pub struct GroqEngine {
    api_key: String,
    base_url: String,
    model: String,
    client: Client,
}

impl GroqEngine {
    /// Convenience constructor using all defaults.
    pub fn new(api_key: impl Into<String>) -> Result<Self, MlError> {
        GroqEngineBuilder::new(api_key).build()
    }

    /// The configured chat model id (e.g. `llama-3.1-8b-instant`).
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    fn chat_url(&self) -> String {
        format!("{}/chat/completions", self.base_url.trim_end_matches('/'))
    }

    /// Issue a chat-completion call. `body` is the OpenAI-shaped
    /// request envelope (model, messages, optional response_format,
    /// max_tokens, temperature). Returns the raw `content` of the
    /// first choice's message.
    async fn chat(&self, endpoint: &'static str, body: Value) -> Result<String, MlError> {
        let resp = self
            .client
            .post(self.chat_url())
            .header(AUTHORIZATION, format!("Bearer {}", self.api_key))
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let raw = resp.text().await?;
        if !status.is_success() {
            return Err(MlError::Upstream {
                endpoint,
                status: status.as_u16(),
                body: truncate(&raw, 512),
            });
        }
        let env: ChatEnvelope = serde_json::from_str(&raw).map_err(|e| MlError::Decode {
            endpoint,
            message: e.to_string(),
            body: truncate(&raw, 512),
        })?;
        let first = env
            .choices
            .into_iter()
            .next()
            .ok_or(MlError::InvalidResponse {
                endpoint,
                message: "choices array is empty".into(),
            })?;
        Ok(first.message.content)
    }
}

#[async_trait]
impl MlEngine for GroqEngine {
    async fn embed(&self, _text: &str) -> Result<Vec<f32>, MlError> {
        Err(MlError::Unsupported("embed"))
    }

    async fn batch_embed(&self, _texts: &[&str]) -> Result<Vec<Vec<f32>>, MlError> {
        Err(MlError::Unsupported("batch_embed"))
    }

    async fn sentiment(&self, text: &str) -> Result<Sentiment, MlError> {
        if text.trim().is_empty() {
            return Err(MlError::EmptyInput("sentiment"));
        }
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {
                    "role": "system",
                    "content": SENTIMENT_SYSTEM_PROMPT,
                },
                {
                    "role": "user",
                    "content": text,
                },
            ],
            "response_format": { "type": "json_object" },
            "max_tokens": 80,
            "temperature": 0.0,
        });
        let content = self.chat("groq.chat.sentiment", body).await?;
        parse_sentiment_response(&content)
    }

    async fn summarize(&self, text: &str, max_tokens: usize) -> Result<String, MlError> {
        if text.trim().is_empty() {
            return Err(MlError::EmptyInput("summarize"));
        }
        // `max_tokens` is the user-facing target. We pass it
        // straight through as the model's hard cap; the prompt also
        // restates the bound so models that pad to fill don't.
        let max_tokens = max_tokens.max(16);
        let system = format!(
            "Summarise the user message in {max_tokens} tokens or fewer. \
             Plain prose, no preamble, no markdown, no bullet points."
        );
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                { "role": "system", "content": system },
                { "role": "user", "content": text },
            ],
            "max_tokens": max_tokens,
            "temperature": 0.2,
        });
        let content = self.chat("groq.chat.summarize", body).await?;
        let trimmed = content.trim().to_string();
        if trimmed.is_empty() {
            return Err(MlError::InvalidResponse {
                endpoint: "groq.chat.summarize",
                message: "model returned empty content".into(),
            });
        }
        Ok(trimmed)
    }

    async fn extract_entities(&self, text: &str) -> Result<Vec<Entity>, MlError> {
        if text.trim().is_empty() {
            return Err(MlError::EmptyInput("extract_entities"));
        }
        let body = serde_json::json!({
            "model": self.model,
            "messages": [
                {
                    "role": "system",
                    "content": ENTITY_SYSTEM_PROMPT,
                },
                {
                    "role": "user",
                    "content": text,
                },
            ],
            "response_format": { "type": "json_object" },
            "max_tokens": 1024,
            "temperature": 0.0,
        });
        let content = self.chat("groq.chat.extract_entities", body).await?;
        parse_entities_response(&content)
    }
}

// ============================================================================
// System prompts (locked verbatim — change only with parity test
// updates so the cache layer doesn't churn keys silently)
// ============================================================================

const SENTIMENT_SYSTEM_PROMPT: &str = concat!(
    "You are a sentiment classifier. Classify the user message as ",
    "positive, negative, or neutral.\n",
    "Respond ONLY with a JSON object of the form ",
    r#"{"label":"positive|negative|neutral","confidence":<float 0-1>}"#,
    ". Do not include any other text."
);

const ENTITY_SYSTEM_PROMPT: &str = concat!(
    "Extract named entities from the user message. Recognised kinds: ",
    "PERSON, ORG, LOC, GPE, EVENT, PRODUCT, DATE, MONEY, NORP, ",
    "FAC, LAW, WORK_OF_ART, LANGUAGE.\n",
    "Respond ONLY with a JSON object of the form ",
    r#"{"entities":[{"text":"...","kind":"PERSON|ORG|...","confidence":<float 0-1>}]}"#,
    ". Use the exact surface form from the input. ",
    "If no entities are present, return {\"entities\":[]}."
);

// ============================================================================
// Response parsers
// ============================================================================

#[derive(Debug, Deserialize)]
struct ChatEnvelope {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ChatMessage {
    content: String,
}

/// Parse the JSON-mode body the sentiment prompt instructs the model
/// to emit. The model occasionally wraps the response in code fences
/// despite the instruction; strip those before decoding.
fn parse_sentiment_response(content: &str) -> Result<Sentiment, MlError> {
    let cleaned = strip_code_fence(content);
    #[derive(Deserialize)]
    struct Body {
        label: String,
        #[serde(default)]
        confidence: Option<f64>,
    }
    let body: Body = serde_json::from_str(cleaned).map_err(|e| MlError::Decode {
        endpoint: "groq.chat.sentiment",
        message: e.to_string(),
        body: truncate(content, 512),
    })?;
    let label = SentimentLabel::parse(&body.label).ok_or(MlError::InvalidResponse {
        endpoint: "groq.chat.sentiment",
        message: format!("unknown label: {}", body.label),
    })?;
    let confidence = body.confidence.map(|c| c.clamp(0.0, 1.0)).unwrap_or(0.5);
    Ok(Sentiment { label, confidence })
}

/// Parse the JSON-mode body the entity prompt instructs the model to
/// emit. Tolerant to the common "wrapped in code fence" failure mode.
fn parse_entities_response(content: &str) -> Result<Vec<Entity>, MlError> {
    let cleaned = strip_code_fence(content);
    #[derive(Deserialize)]
    struct Body {
        #[serde(default)]
        entities: Vec<EntityRow>,
    }
    #[derive(Deserialize)]
    struct EntityRow {
        text: String,
        kind: String,
        #[serde(default)]
        confidence: Option<f64>,
    }
    let body: Body = serde_json::from_str(cleaned).map_err(|e| MlError::Decode {
        endpoint: "groq.chat.extract_entities",
        message: e.to_string(),
        body: truncate(content, 512),
    })?;
    Ok(body
        .entities
        .into_iter()
        .filter(|e| !e.text.trim().is_empty() && !e.kind.trim().is_empty())
        .map(|e| Entity {
            text: e.text.trim().to_string(),
            kind: e.kind.trim().to_uppercase(),
            confidence: e.confidence.map(|c| c.clamp(0.0, 1.0)),
            start: None,
            end: None,
        })
        .collect())
}

/// Strip a leading/trailing markdown code fence (\`\`\`json ... \`\`\`).
/// Models sometimes ignore the "no markdown" instruction.
fn strip_code_fence(s: &str) -> &str {
    let t = s.trim();
    if let Some(stripped) = t
        .strip_prefix("```json")
        .or_else(|| t.strip_prefix("```JSON"))
        .or_else(|| t.strip_prefix("```"))
    {
        return stripped.trim_end_matches("```").trim();
    }
    t
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        let mut end = max;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}…", &s[..end])
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    // ── Builder validation ──────────────────────────────────────

    #[test]
    fn builder_rejects_empty_api_key() {
        let err = GroqEngineBuilder::new("").build().unwrap_err();
        assert!(matches!(err, MlError::MissingConfig("GROQ_API_KEY")));
    }

    #[test]
    fn builder_rejects_empty_model() {
        let err = GroqEngineBuilder::new("k").model("").build().unwrap_err();
        assert!(matches!(err, MlError::MissingConfig("groq_model")));
    }

    #[test]
    fn builder_rejects_empty_base_url() {
        let err = GroqEngineBuilder::new("k")
            .base_url("")
            .build()
            .unwrap_err();
        assert!(matches!(err, MlError::MissingConfig("groq_base_url")));
    }

    #[test]
    fn chat_url_combines_base() {
        let engine = GroqEngineBuilder::new("k")
            .base_url("https://api.test/openai/v1/")
            .build()
            .unwrap();
        assert_eq!(
            engine.chat_url(),
            "https://api.test/openai/v1/chat/completions"
        );
    }

    // ── Pure parser tests ───────────────────────────────────────

    #[test]
    fn parse_sentiment_happy_path() {
        let s = parse_sentiment_response(r#"{"label":"positive","confidence":0.9}"#).unwrap();
        assert_eq!(s.label, SentimentLabel::Positive);
        assert!((s.confidence - 0.9).abs() < 1e-6);
    }

    #[test]
    fn parse_sentiment_strips_code_fence() {
        let s =
            parse_sentiment_response("```json\n{\"label\":\"negative\",\"confidence\":0.7}\n```")
                .unwrap();
        assert_eq!(s.label, SentimentLabel::Negative);
    }

    #[test]
    fn parse_sentiment_clamps_confidence_to_unit_range() {
        let s = parse_sentiment_response(r#"{"label":"neutral","confidence":1.5}"#).unwrap();
        assert!((s.confidence - 1.0).abs() < f64::EPSILON);
        let s = parse_sentiment_response(r#"{"label":"neutral","confidence":-0.2}"#).unwrap();
        assert!(s.confidence.abs() < f64::EPSILON);
    }

    #[test]
    fn parse_sentiment_missing_confidence_defaults_to_half() {
        let s = parse_sentiment_response(r#"{"label":"positive"}"#).unwrap();
        assert!((s.confidence - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn parse_sentiment_unknown_label_is_invalid_response() {
        let err = parse_sentiment_response(r#"{"label":"happy","confidence":1.0}"#).unwrap_err();
        assert!(matches!(err, MlError::InvalidResponse { .. }));
    }

    #[test]
    fn parse_sentiment_invalid_json_is_decode_error() {
        let err = parse_sentiment_response("not json").unwrap_err();
        assert!(matches!(err, MlError::Decode { .. }));
    }

    #[test]
    fn parse_entities_filters_blank_rows_and_uppercases_kinds() {
        let raw = r#"{"entities":[
            {"text":"Tehran","kind":"gpe","confidence":0.9},
            {"text":"","kind":"PERSON"},
            {"text":"Federal Reserve","kind":""},
            {"text":"Jerome Powell","kind":"person"}
        ]}"#;
        let out = parse_entities_response(raw).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].text, "Tehran");
        assert_eq!(out[0].kind, "GPE");
        assert_eq!(out[1].text, "Jerome Powell");
        assert_eq!(out[1].kind, "PERSON");
    }

    #[test]
    fn parse_entities_missing_field_returns_empty_vec() {
        // No `entities` key → default to empty array per #[serde(default)].
        let out = parse_entities_response(r#"{}"#).unwrap();
        assert!(out.is_empty());
    }

    #[test]
    fn strip_code_fence_handles_plain_text() {
        assert_eq!(strip_code_fence("plain"), "plain");
        assert_eq!(strip_code_fence("  spaced  "), "spaced");
        assert_eq!(strip_code_fence("```json\nabc\n```"), "abc");
        assert_eq!(strip_code_fence("```\nabc\n```"), "abc");
    }

    // ── HTTP-level tests ────────────────────────────────────────

    fn chat_response(content: &str) -> serde_json::Value {
        serde_json::json!({
            "choices": [{
                "message": { "role": "assistant", "content": content },
                "finish_reason": "stop"
            }]
        })
    }

    #[tokio::test]
    async fn sentiment_round_trip() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/chat/completions"))
            .and(header("authorization", "Bearer key"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(chat_response(r#"{"label":"positive","confidence":0.85}"#)),
            )
            .mount(&server)
            .await;
        let engine = GroqEngineBuilder::new("key")
            .base_url(server.uri())
            .build()
            .unwrap();
        let s = engine.sentiment("the new release is great").await.unwrap();
        assert_eq!(s.label, SentimentLabel::Positive);
        assert!((s.confidence - 0.85).abs() < 1e-6);
    }

    #[tokio::test]
    async fn summarize_returns_trimmed_content() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(chat_response("\n  Iran tested a missile.\n  ")),
            )
            .mount(&server)
            .await;
        let engine = GroqEngineBuilder::new("key")
            .base_url(server.uri())
            .build()
            .unwrap();
        let s = engine.summarize("body of article", 80).await.unwrap();
        assert_eq!(s, "Iran tested a missile.");
    }

    #[tokio::test]
    async fn summarize_empty_response_is_invalid_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(chat_response("   ")))
            .mount(&server)
            .await;
        let engine = GroqEngineBuilder::new("key")
            .base_url(server.uri())
            .build()
            .unwrap();
        let err = engine.summarize("text", 80).await.unwrap_err();
        assert!(matches!(err, MlError::InvalidResponse { .. }));
    }

    #[tokio::test]
    async fn extract_entities_round_trip() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(chat_response(
                r#"{"entities":[{"text":"Tehran","kind":"GPE","confidence":0.9}]}"#,
            )))
            .mount(&server)
            .await;
        let engine = GroqEngineBuilder::new("key")
            .base_url(server.uri())
            .build()
            .unwrap();
        let entities = engine
            .extract_entities("missile fired at Tehran")
            .await
            .unwrap();
        assert_eq!(entities.len(), 1);
        assert_eq!(entities[0].text, "Tehran");
        assert_eq!(entities[0].kind, "GPE");
        assert_eq!(entities[0].confidence, Some(0.9));
    }

    #[tokio::test]
    async fn upstream_5xx_propagates_status_and_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(500).set_body_string("internal"))
            .mount(&server)
            .await;
        let engine = GroqEngineBuilder::new("key")
            .base_url(server.uri())
            .build()
            .unwrap();
        let err = engine.sentiment("hi").await.unwrap_err();
        match err {
            MlError::Upstream { status, body, .. } => {
                assert_eq!(status, 500);
                assert!(body.contains("internal"));
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn empty_choices_array_is_invalid_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"choices": []})),
            )
            .mount(&server)
            .await;
        let engine = GroqEngineBuilder::new("key")
            .base_url(server.uri())
            .build()
            .unwrap();
        let err = engine.sentiment("hi").await.unwrap_err();
        assert!(matches!(err, MlError::InvalidResponse { .. }));
    }

    #[tokio::test]
    async fn embed_methods_unsupported_on_groq() {
        let engine = GroqEngineBuilder::new("k").build().unwrap();
        assert!(matches!(
            engine.embed("x").await,
            Err(MlError::Unsupported("embed"))
        ));
        assert!(matches!(
            engine.batch_embed(&["x"]).await,
            Err(MlError::Unsupported("batch_embed"))
        ));
    }

    #[tokio::test]
    async fn empty_input_validation() {
        let engine = GroqEngineBuilder::new("k").build().unwrap();
        assert!(matches!(
            engine.sentiment("   ").await,
            Err(MlError::EmptyInput("sentiment"))
        ));
        assert!(matches!(
            engine.summarize("", 100).await,
            Err(MlError::EmptyInput("summarize"))
        ));
        assert!(matches!(
            engine.extract_entities("   \n").await,
            Err(MlError::EmptyInput("extract_entities"))
        ));
    }
}
