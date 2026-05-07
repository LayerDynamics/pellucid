//! HuggingFace Inference API embedder.
//!
//! Hits the public Feature-Extraction pipeline endpoint:
//!
//! ```text
//! POST https://api-inference.huggingface.co/pipeline/feature-extraction/{model}
//! Authorization: Bearer <HF_TOKEN>
//! Content-Type: application/json
//! { "inputs": "text" }   // or { "inputs": ["t1", "t2"] }
//! ```
//!
//! For sentence-transformer models (`sentence-transformers/*`) the
//! response body is a single sentence-pooled vector (`[f32]`) for
//! single inputs and a list of vectors (`[[f32]]`) for batch inputs.
//! For non-pooled models the same endpoint returns token-level
//! embeddings — those are NOT supported by this engine; pick a
//! sentence-transformer model.
//!
//! Sentiment / summarization / NER are not implemented here — the
//! HF Inference API does support them via different pipeline names,
//! but the project's chat-style backends ship in [`crate::groq`]. A
//! caller wanting "all HF" can wire a separate HF backend per task
//! in [`crate::CompositeEngine`].

use async_trait::async_trait;
use reqwest::header::{AUTHORIZATION, CONTENT_TYPE};
use reqwest::Client;
use serde_json::Value;

use crate::engine::MlEngine;
use crate::types::{Entity, MlError, Sentiment};

const DEFAULT_BASE_URL: &str = "https://api-inference.huggingface.co";

/// Default sentence-transformer model. 384-dim, fast, free-tier
/// friendly. Switch via [`HfEmbeddingEngineBuilder::model`] when a
/// different vector dimension is required (e.g. `BAAI/bge-large-en-v1.5`
/// at 1024-dim for higher recall at the cost of bigger sqlite-vec
/// indexes).
pub const DEFAULT_MODEL: &str = "sentence-transformers/all-MiniLM-L6-v2";

/// Builder for [`HfEmbeddingEngine`] — lets the caller override the
/// model + base URL (the latter for `wiremock`-backed tests).
#[derive(Debug, Clone)]
pub struct HfEmbeddingEngineBuilder {
    api_token: String,
    base_url: String,
    model: String,
    client: Option<Client>,
}

impl HfEmbeddingEngineBuilder {
    /// Construct with a non-empty API token. Empty tokens fail at
    /// `build()` rather than being silently accepted (HF returns 401
    /// later, but the operator should know at boot).
    #[must_use]
    pub fn new(api_token: impl Into<String>) -> Self {
        Self {
            api_token: api_token.into(),
            base_url: DEFAULT_BASE_URL.to_string(),
            model: DEFAULT_MODEL.to_string(),
            client: None,
        }
    }

    /// Override the embedding model. Must be a sentence-transformer
    /// model id on HuggingFace.
    #[must_use]
    pub fn model(mut self, model: impl Into<String>) -> Self {
        self.model = model.into();
        self
    }

    /// Override the base URL. Tests use this to point at a
    /// `wiremock` mock server.
    #[must_use]
    pub fn base_url(mut self, url: impl Into<String>) -> Self {
        self.base_url = url.into();
        self
    }

    /// Override the underlying HTTP client. Useful when the host
    /// already manages a single shared client with custom timeouts /
    /// connection pool sizing.
    #[must_use]
    pub fn http_client(mut self, client: Client) -> Self {
        self.client = Some(client);
        self
    }

    /// Build the engine. Returns
    /// [`MlError::MissingConfig("HF_TOKEN")`] if the token is empty
    /// so `pellucid-edge-bin` can fail closed at boot.
    pub fn build(self) -> Result<HfEmbeddingEngine, MlError> {
        if self.api_token.trim().is_empty() {
            return Err(MlError::MissingConfig("HF_TOKEN"));
        }
        if self.model.trim().is_empty() {
            return Err(MlError::MissingConfig("hf_model"));
        }
        if self.base_url.trim().is_empty() {
            return Err(MlError::MissingConfig("hf_base_url"));
        }
        let client = self.client.unwrap_or_default();
        Ok(HfEmbeddingEngine {
            api_token: self.api_token,
            base_url: self.base_url,
            model: self.model,
            client,
        })
    }
}

/// HuggingFace-backed embedder.
#[derive(Debug, Clone)]
pub struct HfEmbeddingEngine {
    api_token: String,
    base_url: String,
    model: String,
    client: Client,
}

impl HfEmbeddingEngine {
    /// Convenience constructor — equivalent to
    /// `HfEmbeddingEngineBuilder::new(token).build()`. Uses the
    /// default model + base URL.
    pub fn new(api_token: impl Into<String>) -> Result<Self, MlError> {
        HfEmbeddingEngineBuilder::new(api_token).build()
    }

    /// The configured model id (e.g.
    /// `sentence-transformers/all-MiniLM-L6-v2`). Exposed so callers
    /// can persist it alongside indexed vectors and detect dim
    /// mismatches at query time.
    #[must_use]
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Build the full POST URL for a feature-extraction call.
    fn endpoint_url(&self) -> String {
        format!(
            "{}/pipeline/feature-extraction/{}",
            self.base_url.trim_end_matches('/'),
            self.model
        )
    }
}

#[async_trait]
impl MlEngine for HfEmbeddingEngine {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, MlError> {
        if text.is_empty() {
            return Err(MlError::EmptyInput("embed"));
        }
        let url = self.endpoint_url();
        let body = serde_json::json!({ "inputs": text });
        let resp = self
            .client
            .post(&url)
            .header(AUTHORIZATION, format!("Bearer {}", self.api_token))
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let raw = resp.text().await?;
        if !status.is_success() {
            return Err(MlError::Upstream {
                endpoint: "hf.feature-extraction",
                status: status.as_u16(),
                body: truncate(&raw, 512),
            });
        }
        parse_single_embedding(&raw)
    }

    async fn batch_embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, MlError> {
        if texts.iter().any(|t| t.is_empty()) {
            return Err(MlError::EmptyInput("batch_embed"));
        }
        if texts.is_empty() {
            return Ok(Vec::new());
        }

        let url = self.endpoint_url();
        let body = serde_json::json!({ "inputs": texts });
        let resp = self
            .client
            .post(&url)
            .header(AUTHORIZATION, format!("Bearer {}", self.api_token))
            .header(CONTENT_TYPE, "application/json")
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        let raw = resp.text().await?;
        if !status.is_success() {
            return Err(MlError::Upstream {
                endpoint: "hf.feature-extraction.batch",
                status: status.as_u16(),
                body: truncate(&raw, 512),
            });
        }
        let parsed = parse_batch_embeddings(&raw)?;
        if parsed.len() != texts.len() {
            return Err(MlError::InvalidResponse {
                endpoint: "hf.feature-extraction.batch",
                message: format!(
                    "expected {} vectors, upstream returned {}",
                    texts.len(),
                    parsed.len()
                ),
            });
        }
        Ok(parsed)
    }

    async fn sentiment(&self, _text: &str) -> Result<Sentiment, MlError> {
        Err(MlError::Unsupported("sentiment"))
    }

    async fn summarize(&self, _text: &str, _max_tokens: usize) -> Result<String, MlError> {
        Err(MlError::Unsupported("summarize"))
    }

    async fn extract_entities(&self, _text: &str) -> Result<Vec<Entity>, MlError> {
        Err(MlError::Unsupported("extract_entities"))
    }
}

// ============================================================================
// Response parsers
// ============================================================================

/// Parse the body of a single-input feature-extraction call.
///
/// HF can return one of:
/// - `[f32, f32, …]` — sentence-pooled vector (the happy path for
///   `sentence-transformers/*`)
/// - `[[f32, …], [f32, …], …]` — token-level embeddings, NOT
///   supported here; we emit `InvalidResponse` so callers don't
///   silently get back the wrong shape.
fn parse_single_embedding(raw: &str) -> Result<Vec<f32>, MlError> {
    let value: Value = serde_json::from_str(raw).map_err(|e| MlError::Decode {
        endpoint: "hf.feature-extraction",
        message: e.to_string(),
        body: truncate(raw, 512),
    })?;

    // Some sentence-transformer models on HF return `[[f32; D]]`
    // (one row per input) even for a single string input — collapse
    // that down to the inner row.
    let inner = match value {
        Value::Array(mut outer) => match outer.first() {
            Some(Value::Array(_)) if outer.len() == 1 => outer.remove(0),
            _ => Value::Array(outer),
        },
        other => {
            return Err(MlError::InvalidResponse {
                endpoint: "hf.feature-extraction",
                message: format!("expected JSON array, got {}", json_type_name(&other)),
            });
        }
    };

    let arr = match inner {
        Value::Array(a) => a,
        other => {
            return Err(MlError::InvalidResponse {
                endpoint: "hf.feature-extraction",
                message: format!("expected JSON array, got {}", json_type_name(&other)),
            });
        }
    };
    if arr.is_empty() {
        return Err(MlError::InvalidResponse {
            endpoint: "hf.feature-extraction",
            message: "embedding is empty".into(),
        });
    }
    // Reject token-level outputs (an array of arrays). Sentence
    // transformers must return a flat vector.
    if matches!(arr.first(), Some(Value::Array(_))) {
        return Err(MlError::InvalidResponse {
            endpoint: "hf.feature-extraction",
            message: "got token-level embeddings — pick a sentence-transformer model".into(),
        });
    }
    let mut out = Vec::with_capacity(arr.len());
    for v in arr {
        match v.as_f64() {
            Some(n) => out.push(n as f32),
            None => {
                return Err(MlError::InvalidResponse {
                    endpoint: "hf.feature-extraction",
                    message: format!("non-numeric element in vector: {v:?}"),
                });
            }
        }
    }
    Ok(out)
}

/// Parse the body of a batch (`inputs: [..]`) feature-extraction
/// call. Expects `[[f32; D], …]`.
fn parse_batch_embeddings(raw: &str) -> Result<Vec<Vec<f32>>, MlError> {
    let value: Value = serde_json::from_str(raw).map_err(|e| MlError::Decode {
        endpoint: "hf.feature-extraction.batch",
        message: e.to_string(),
        body: truncate(raw, 512),
    })?;
    let outer = match value {
        Value::Array(a) => a,
        other => {
            return Err(MlError::InvalidResponse {
                endpoint: "hf.feature-extraction.batch",
                message: format!("expected JSON array, got {}", json_type_name(&other)),
            });
        }
    };
    let mut rows = Vec::with_capacity(outer.len());
    for row in outer {
        let arr = match row {
            Value::Array(a) => a,
            other => {
                return Err(MlError::InvalidResponse {
                    endpoint: "hf.feature-extraction.batch",
                    message: format!("row is not an array: {}", json_type_name(&other)),
                });
            }
        };
        let mut vec = Vec::with_capacity(arr.len());
        for v in arr {
            match v.as_f64() {
                Some(n) => vec.push(n as f32),
                None => {
                    return Err(MlError::InvalidResponse {
                        endpoint: "hf.feature-extraction.batch",
                        message: format!("non-numeric element: {v:?}"),
                    });
                }
            }
        }
        rows.push(vec);
    }
    Ok(rows)
}

fn json_type_name(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "bool",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
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

    #[test]
    fn builder_rejects_empty_token() {
        let err = HfEmbeddingEngineBuilder::new("").build().unwrap_err();
        match err {
            MlError::MissingConfig(name) => assert_eq!(name, "HF_TOKEN"),
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn builder_rejects_empty_model() {
        let err = HfEmbeddingEngineBuilder::new("token")
            .model("")
            .build()
            .unwrap_err();
        match err {
            MlError::MissingConfig(name) => assert_eq!(name, "hf_model"),
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn builder_rejects_empty_base_url() {
        let err = HfEmbeddingEngineBuilder::new("token")
            .base_url("")
            .build()
            .unwrap_err();
        match err {
            MlError::MissingConfig(name) => assert_eq!(name, "hf_base_url"),
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn endpoint_url_combines_base_and_model() {
        let engine = HfEmbeddingEngineBuilder::new("token")
            .base_url("https://example.test/")
            .model("sentence-transformers/all-MiniLM-L6-v2")
            .build()
            .unwrap();
        assert_eq!(
            engine.endpoint_url(),
            "https://example.test/pipeline/feature-extraction/sentence-transformers/all-MiniLM-L6-v2"
        );
    }

    #[test]
    fn parse_single_accepts_flat_vector() {
        let v = parse_single_embedding("[0.1, 0.2, -0.3]").unwrap();
        assert_eq!(v.len(), 3);
        assert!((v[0] - 0.1).abs() < 1e-6);
        assert!((v[2] - (-0.3)).abs() < 1e-6);
    }

    #[test]
    fn parse_single_unwraps_one_row_batch() {
        // HF sometimes wraps single-input responses in an outer
        // array; we collapse `[[..]]` → `[..]`.
        let v = parse_single_embedding("[[0.1, 0.2]]").unwrap();
        assert_eq!(v, vec![0.1, 0.2]);
    }

    #[test]
    fn parse_single_rejects_token_level_output() {
        // A multi-row array (token-level embeddings) is the wrong
        // shape for an embedder.
        let err = parse_single_embedding("[[0.1, 0.2], [0.3, 0.4]]").unwrap_err();
        match err {
            MlError::InvalidResponse { message, .. } => {
                assert!(message.contains("token-level"));
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[test]
    fn parse_single_rejects_empty_array() {
        let err = parse_single_embedding("[]").unwrap_err();
        assert!(matches!(err, MlError::InvalidResponse { .. }));
    }

    #[test]
    fn parse_single_rejects_non_array() {
        let err = parse_single_embedding(r#""nope""#).unwrap_err();
        assert!(matches!(err, MlError::InvalidResponse { .. }));
    }

    #[test]
    fn parse_single_rejects_invalid_json() {
        let err = parse_single_embedding("not json").unwrap_err();
        assert!(matches!(err, MlError::Decode { .. }));
    }

    #[test]
    fn parse_batch_accepts_array_of_arrays() {
        let rows = parse_batch_embeddings("[[0.1, 0.2], [0.3, 0.4], [0.5, 0.6]]").unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0], vec![0.1, 0.2]);
        assert_eq!(rows[2], vec![0.5, 0.6]);
    }

    #[test]
    fn parse_batch_rejects_non_array_row() {
        let err = parse_batch_embeddings(r#"[[0.1, 0.2], "nope"]"#).unwrap_err();
        assert!(matches!(err, MlError::InvalidResponse { .. }));
    }

    // ── HTTP-level tests against wiremock ────────────────────────

    #[tokio::test]
    async fn embed_sends_bearer_and_returns_vector_on_200() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path(
                "/pipeline/feature-extraction/sentence-transformers/all-MiniLM-L6-v2",
            ))
            .and(header("authorization", "Bearer secret-token"))
            .and(header("content-type", "application/json"))
            .respond_with(ResponseTemplate::new(200).set_body_string("[0.11, 0.22, 0.33, 0.44]"))
            .mount(&server)
            .await;

        let engine = HfEmbeddingEngineBuilder::new("secret-token")
            .base_url(server.uri())
            .build()
            .unwrap();
        let v = engine.embed("hello world").await.unwrap();
        assert_eq!(v.len(), 4);
        assert!((v[0] - 0.11).abs() < 1e-5);
        assert!((v[3] - 0.44).abs() < 1e-5);
    }

    #[tokio::test]
    async fn embed_returns_empty_input_error_for_empty_string() {
        // Builder is fine — only call-time empty input fails.
        let engine = HfEmbeddingEngineBuilder::new("token").build().unwrap();
        let err = engine.embed("").await.unwrap_err();
        assert!(matches!(err, MlError::EmptyInput("embed")));
    }

    #[tokio::test]
    async fn embed_maps_5xx_to_upstream_error_with_body() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(503).set_body_string("model is loading"))
            .mount(&server)
            .await;
        let engine = HfEmbeddingEngineBuilder::new("token")
            .base_url(server.uri())
            .build()
            .unwrap();
        let err = engine.embed("hi").await.unwrap_err();
        match err {
            MlError::Upstream { status, body, .. } => {
                assert_eq!(status, 503);
                assert!(body.contains("model is loading"));
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn batch_embed_sends_array_input_and_parses_array_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("[[0.1, 0.2], [0.3, 0.4], [0.5, 0.6]]"),
            )
            .mount(&server)
            .await;
        let engine = HfEmbeddingEngineBuilder::new("token")
            .base_url(server.uri())
            .build()
            .unwrap();
        let rows = engine.batch_embed(&["a", "b", "c"]).await.unwrap();
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[1], vec![0.3, 0.4]);
    }

    #[tokio::test]
    async fn batch_embed_size_mismatch_is_invalid_response() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_string("[[0.1, 0.2]]"))
            .mount(&server)
            .await;
        let engine = HfEmbeddingEngineBuilder::new("token")
            .base_url(server.uri())
            .build()
            .unwrap();
        let err = engine.batch_embed(&["a", "b"]).await.unwrap_err();
        match err {
            MlError::InvalidResponse { message, .. } => {
                assert!(message.contains("expected 2"));
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn sentiment_summarize_extract_entities_are_unsupported() {
        let engine = HfEmbeddingEngineBuilder::new("token").build().unwrap();
        assert!(matches!(
            engine.sentiment("x").await,
            Err(MlError::Unsupported("sentiment"))
        ));
        assert!(matches!(
            engine.summarize("x", 100).await,
            Err(MlError::Unsupported("summarize"))
        ));
        assert!(matches!(
            engine.extract_entities("x").await,
            Err(MlError::Unsupported("extract_entities"))
        ));
    }
}
