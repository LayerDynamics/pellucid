//! Tauri-host MlEngine wiring + IPC commands.
//!
//! The desktop host owns its own [`pellucid_ml::MlEngine`] instance
//! built from the `KeychainVault` (stores `groq_api_key` and
//! `hf_token` alongside the existing telegram fields). The webview
//! reaches this engine through five `#[tauri::command]` handlers
//! mirroring the public RPC surface (`/api/intelligence/v1/*`):
//!
//! - `ml_embed`              → [`pellucid_ml::MlEngine::embed`]
//! - `ml_batch_embed`        → [`pellucid_ml::MlEngine::batch_embed`]
//! - `ml_sentiment`          → [`pellucid_ml::MlEngine::sentiment`]
//! - `ml_summarize`          → [`pellucid_ml::MlEngine::summarize`]
//! - `ml_extract_entities`   → [`pellucid_ml::MlEngine::extract_entities`]
//!
//! The engine is built lazily on first call: the keychain fetch +
//! HTTP-client wire-up costs ~10 ms and only happens when the
//! webview first invokes an ML command, so cold-launch isn't
//! taxed for users that never open an intelligence panel. The
//! result is cached behind a `Mutex<Option<...>>` and reused for
//! the process lifetime; vault changes that swap the keys
//! invalidate the cached engine via [`MlEngineState::reset`]
//! (called by the `refresh_secrets` IPC handler when it observes
//! a vault `Updated` event).
//!
//! All five handlers return shaped error envelopes matching the
//! webview's expectations — never bubble a raw `MlError` Debug
//! string into the IPC reply.

use std::sync::Arc;

use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::sync::Mutex;

use pellucid_core::vault::Vault;
use pellucid_ml::{
    build_from_vault, Entity, FromVaultError, MlEngine, MlError, Sentiment, SentimentLabel,
    VaultEngineConfig,
};

/// Lazily-constructed engine cache. The webview hits this through
/// `tauri::State<MlEngineState>` — first call builds the engine, all
/// subsequent calls reuse it.
#[derive(Default)]
pub struct MlEngineState {
    cached: Mutex<Option<Arc<dyn MlEngine>>>,
    config: VaultEngineConfig,
}

impl std::fmt::Debug for MlEngineState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MlEngineState")
            .field("cached", &"Mutex<Option<Arc<dyn MlEngine>>>")
            .field("hf_model", &self.config.resolved_hf_model())
            .field("groq_model", &self.config.resolved_groq_model())
            .finish()
    }
}

impl MlEngineState {
    /// Construct with library defaults
    /// (`sentence-transformers/all-MiniLM-L6-v2` + `llama-3.1-8b-instant`).
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Construct with caller-chosen model overrides.
    #[must_use]
    pub fn with_config(config: VaultEngineConfig) -> Self {
        Self {
            cached: Mutex::new(None),
            config,
        }
    }

    /// Drop the cached engine — next call rebuilds from the
    /// freshest `Vault` read. Used after a `VaultChange::Updated`
    /// event so a key-swap in the keychain takes effect without
    /// requiring an app restart.
    pub async fn reset(&self) {
        let mut guard = self.cached.lock().await;
        *guard = None;
    }

    /// Get the cached engine if any. Test-only — production code
    /// goes through [`Self::engine`] which lazily builds on first
    /// call.
    #[cfg(test)]
    pub async fn cached(&self) -> Option<Arc<dyn MlEngine>> {
        self.cached.lock().await.clone()
    }

    /// Borrow (lazily-build) the engine. Returns the cached
    /// instance if one exists, else builds via
    /// `pellucid_ml::build_from_vault(vault, &self.config)` and
    /// stores it.
    pub async fn engine(&self, vault: &dyn Vault) -> Result<Arc<dyn MlEngine>, MlIpcError> {
        let mut guard = self.cached.lock().await;
        if let Some(engine) = guard.as_ref() {
            return Ok(Arc::clone(engine));
        }
        let engine = build_from_vault(vault, &self.config)
            .await
            .map_err(MlIpcError::from)?;
        *guard = Some(Arc::clone(&engine));
        Ok(engine)
    }
}

/// IPC-facing error envelope. Distinct from
/// [`pellucid_ml::MlError`] because the webview only needs a small
/// stable code/message shape — never the raw transport error.
#[derive(Debug, Error, Serialize)]
#[serde(tag = "code", content = "message", rename_all = "snake_case")]
pub enum MlIpcError {
    /// Required key (`HF_TOKEN` / `GROQ_API_KEY`) is missing in the
    /// keychain. Webview should surface a "configure ML provider"
    /// prompt that drives the secrets editor.
    #[error("missing key: {0}")]
    MissingKey(String),

    /// The keychain read failed (locked, permission denied, etc).
    #[error("vault: {0}")]
    Vault(String),

    /// Caller-supplied input was empty / too long / otherwise
    /// invalid before the upstream was even contacted.
    #[error("invalid input: {0}")]
    InvalidInput(String),

    /// HF / Groq returned a non-2xx status. `status` is the upstream
    /// HTTP code so the webview can render a tier-specific message
    /// (e.g. 429 → "rate limited, retrying", 5xx → "service down").
    #[error("upstream {status}: {message}")]
    Upstream { status: u16, message: String },

    /// Upstream returned 2xx but the body didn't parse to the
    /// expected shape. Almost always a model-config drift that
    /// requires our side to update; surface as "ml provider error".
    #[error("decode: {0}")]
    Decode(String),

    /// Any other failure that doesn't fit the categories above —
    /// should never appear in normal operation, but covers
    /// otherwise-uncategorised `MlError::Http` etc.
    #[error("internal: {0}")]
    Internal(String),
}

impl From<FromVaultError> for MlIpcError {
    fn from(e: FromVaultError) -> Self {
        match e {
            FromVaultError::MissingKey(k) => Self::MissingKey(k.to_string()),
            FromVaultError::Vault(v) => Self::Vault(v.to_string()),
            FromVaultError::Engine(m) => Self::from(m),
        }
    }
}

impl From<MlError> for MlIpcError {
    fn from(e: MlError) -> Self {
        match e {
            MlError::MissingConfig(name) => Self::MissingKey(name.to_string()),
            MlError::EmptyInput(op) => Self::InvalidInput(format!("empty input for `{op}`")),
            MlError::Unsupported(op) => Self::InvalidInput(format!(
                "operation `{op}` not supported by configured backend"
            )),
            MlError::Upstream {
                endpoint: _,
                status,
                body,
            } => Self::Upstream {
                status,
                message: body,
            },
            MlError::Decode { message, .. } | MlError::InvalidResponse { message, .. } => {
                Self::Decode(message)
            }
            MlError::Http(err) => Self::Internal(err.to_string()),
        }
    }
}

// ============================================================================
// IPC command request / response shapes
// ============================================================================

#[derive(Debug, Deserialize)]
pub struct EmbedArgs {
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct EmbedResponse {
    pub vector: Vec<f32>,
    /// Concrete model id used (e.g.
    /// `sentence-transformers/all-MiniLM-L6-v2`). Returned so the
    /// webview can store it alongside indexed vectors and refuse to
    /// query against a corpus indexed with a different dimension.
    pub model: String,
}

#[derive(Debug, Deserialize)]
pub struct BatchEmbedArgs {
    pub texts: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct BatchEmbedResponse {
    pub vectors: Vec<Vec<f32>>,
    pub model: String,
}

#[derive(Debug, Deserialize)]
pub struct SentimentArgs {
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct SentimentResponse {
    pub label: SentimentLabel,
    pub confidence: f64,
}

#[derive(Debug, Deserialize)]
pub struct SummarizeArgs {
    pub text: String,
    /// Optional cap, defaults to 200, clamped to `[16, 1024]`.
    #[serde(default)]
    pub max_tokens: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct SummarizeResponse {
    pub summary: String,
}

#[derive(Debug, Deserialize)]
pub struct ExtractEntitiesArgs {
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct ExtractEntitiesResponse {
    pub entities: Vec<Entity>,
}

// ============================================================================
// Shared validation
// ============================================================================

/// 16 KiB hard cap on single-call inputs — same as the edge
/// handlers. `summarize` overrides up to 64 KiB via
/// [`MAX_INPUT_BYTES_SUMMARIZE`].
pub const MAX_INPUT_BYTES: usize = 16 * 1024;

/// 64 KiB cap for `summarize` — articles tend to be longer than
/// sentiment / entity inputs.
pub const MAX_INPUT_BYTES_SUMMARIZE: usize = 64 * 1024;

const DEFAULT_SUMMARIZE_TOKENS: usize = 200;
const MIN_SUMMARIZE_TOKENS: usize = 16;
const MAX_SUMMARIZE_TOKENS: usize = 1024;

fn validate_input<'a>(
    text: &'a str,
    op: &'static str,
    max_bytes: usize,
) -> Result<&'a str, MlIpcError> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(MlIpcError::InvalidInput(format!("empty input for `{op}`")));
    }
    if text.len() > max_bytes {
        return Err(MlIpcError::InvalidInput(format!(
            "input for `{op}` exceeds {max_bytes} bytes"
        )));
    }
    Ok(trimmed)
}

// ============================================================================
// Handler bodies — separated from the `#[tauri::command]` wrappers
// so unit tests can exercise the engine logic without spinning up
// a Tauri runtime.
// ============================================================================

pub async fn handle_embed(
    state: &MlEngineState,
    vault: &dyn Vault,
    args: EmbedArgs,
) -> Result<EmbedResponse, MlIpcError> {
    let trimmed = validate_input(&args.text, "embed", MAX_INPUT_BYTES)?;
    let engine = state.engine(vault).await?;
    let vector = engine.embed(trimmed).await?;
    Ok(EmbedResponse {
        vector,
        model: state.config.resolved_hf_model().to_string(),
    })
}

pub async fn handle_batch_embed(
    state: &MlEngineState,
    vault: &dyn Vault,
    args: BatchEmbedArgs,
) -> Result<BatchEmbedResponse, MlIpcError> {
    if args.texts.is_empty() {
        return Err(MlIpcError::InvalidInput(
            "empty texts array for `batch_embed`".into(),
        ));
    }
    for (i, t) in args.texts.iter().enumerate() {
        if t.trim().is_empty() {
            return Err(MlIpcError::InvalidInput(format!("texts[{i}] is empty")));
        }
        if t.len() > MAX_INPUT_BYTES {
            return Err(MlIpcError::InvalidInput(format!(
                "texts[{i}] exceeds {MAX_INPUT_BYTES} bytes"
            )));
        }
    }
    let engine = state.engine(vault).await?;
    let refs: Vec<&str> = args.texts.iter().map(String::as_str).collect();
    let vectors = engine.batch_embed(&refs).await?;
    Ok(BatchEmbedResponse {
        vectors,
        model: state.config.resolved_hf_model().to_string(),
    })
}

pub async fn handle_sentiment(
    state: &MlEngineState,
    vault: &dyn Vault,
    args: SentimentArgs,
) -> Result<SentimentResponse, MlIpcError> {
    let trimmed = validate_input(&args.text, "sentiment", MAX_INPUT_BYTES)?;
    let engine = state.engine(vault).await?;
    let s: Sentiment = engine.sentiment(trimmed).await?;
    Ok(SentimentResponse {
        label: s.label,
        confidence: s.confidence,
    })
}

pub async fn handle_summarize(
    state: &MlEngineState,
    vault: &dyn Vault,
    args: SummarizeArgs,
) -> Result<SummarizeResponse, MlIpcError> {
    let trimmed = validate_input(&args.text, "summarize", MAX_INPUT_BYTES_SUMMARIZE)?;
    let max_tokens = args
        .max_tokens
        .unwrap_or(DEFAULT_SUMMARIZE_TOKENS)
        .clamp(MIN_SUMMARIZE_TOKENS, MAX_SUMMARIZE_TOKENS);
    let engine = state.engine(vault).await?;
    let summary = engine.summarize(trimmed, max_tokens).await?;
    Ok(SummarizeResponse { summary })
}

pub async fn handle_extract_entities(
    state: &MlEngineState,
    vault: &dyn Vault,
    args: ExtractEntitiesArgs,
) -> Result<ExtractEntitiesResponse, MlIpcError> {
    let trimmed = validate_input(&args.text, "extract_entities", MAX_INPUT_BYTES)?;
    let engine = state.engine(vault).await?;
    let entities = engine.extract_entities(trimmed).await?;
    Ok(ExtractEntitiesResponse { entities })
}

// ============================================================================
// Tests — exercise validation, error mapping, and engine caching.
// ============================================================================

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use pellucid_core::vault::{InMemoryVault, SecretsBlob};

    #[tokio::test]
    async fn engine_returns_missing_key_when_vault_is_empty() {
        let v = InMemoryVault::new();
        let state = MlEngineState::new();
        // `Arc<dyn MlEngine>` doesn't impl `Debug`, so the
        // `Result::unwrap_err` shorthand can't be used; `let-else`
        // is the cleanest way to extract the error.
        let res = state.engine(&v).await;
        let Err(err) = res else {
            panic!("expected MissingKey error, got Ok")
        };
        match err {
            MlIpcError::MissingKey(name) => {
                assert!(
                    name == "HF_TOKEN" || name == "GROQ_API_KEY",
                    "unexpected key: {name}"
                );
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn engine_caches_after_first_build() {
        let v = InMemoryVault::new();
        v.write(&SecretsBlob {
            groq_api_key: Some("g".into()),
            hf_token: Some("h".into()),
            ..Default::default()
        })
        .await
        .unwrap();
        let state = MlEngineState::new();
        let first = state.engine(&v).await.unwrap();
        let second = state.engine(&v).await.unwrap();
        // Same Arc pointee.
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[tokio::test]
    async fn reset_drops_cached_engine() {
        let v = InMemoryVault::new();
        v.write(&SecretsBlob {
            groq_api_key: Some("g".into()),
            hf_token: Some("h".into()),
            ..Default::default()
        })
        .await
        .unwrap();
        let state = MlEngineState::new();
        let _ = state.engine(&v).await.unwrap();
        assert!(state.cached().await.is_some());
        state.reset().await;
        assert!(state.cached().await.is_none());
    }

    #[tokio::test]
    async fn validate_rejects_empty_after_trim() {
        let err = validate_input("   \n", "embed", 1000).unwrap_err();
        assert!(matches!(err, MlIpcError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn validate_rejects_oversize_input() {
        let big = "x".repeat(MAX_INPUT_BYTES + 1);
        let err = validate_input(&big, "embed", MAX_INPUT_BYTES).unwrap_err();
        assert!(matches!(err, MlIpcError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn batch_embed_rejects_empty_array() {
        let v = InMemoryVault::new();
        v.write(&SecretsBlob {
            groq_api_key: Some("g".into()),
            hf_token: Some("h".into()),
            ..Default::default()
        })
        .await
        .unwrap();
        let state = MlEngineState::new();
        let err = handle_batch_embed(&state, &v, BatchEmbedArgs { texts: vec![] })
            .await
            .unwrap_err();
        assert!(matches!(err, MlIpcError::InvalidInput(_)));
    }

    #[tokio::test]
    async fn batch_embed_rejects_empty_element() {
        let v = InMemoryVault::new();
        v.write(&SecretsBlob {
            groq_api_key: Some("g".into()),
            hf_token: Some("h".into()),
            ..Default::default()
        })
        .await
        .unwrap();
        let state = MlEngineState::new();
        let err = handle_batch_embed(
            &state,
            &v,
            BatchEmbedArgs {
                texts: vec!["good".into(), "  ".into()],
            },
        )
        .await
        .unwrap_err();
        match err {
            MlIpcError::InvalidInput(msg) => assert!(msg.contains("texts[1]")),
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn ml_error_upstream_maps_to_ipc_upstream() {
        let m = MlError::Upstream {
            endpoint: "groq.chat.sentiment",
            status: 429,
            body: "rate limit".into(),
        };
        let i: MlIpcError = m.into();
        match i {
            MlIpcError::Upstream { status, message } => {
                assert_eq!(status, 429);
                assert_eq!(message, "rate limit");
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn ml_error_missing_config_maps_to_missing_key() {
        let m = MlError::MissingConfig("HF_TOKEN");
        let i: MlIpcError = m.into();
        assert!(matches!(i, MlIpcError::MissingKey(ref s) if s == "HF_TOKEN"));
    }

    #[tokio::test]
    async fn ml_error_decode_maps_to_decode() {
        let m = MlError::Decode {
            endpoint: "hf.feature-extraction",
            message: "unexpected eof".into(),
            body: "{".into(),
        };
        let i: MlIpcError = m.into();
        assert!(matches!(i, MlIpcError::Decode(ref s) if s.contains("unexpected eof")));
    }

    #[tokio::test]
    async fn from_vault_error_maps_to_ipc() {
        let e = FromVaultError::MissingKey("GROQ_API_KEY");
        let i: MlIpcError = e.into();
        assert!(matches!(i, MlIpcError::MissingKey(ref s) if s == "GROQ_API_KEY"));
    }

    /// Stub backend used to drive `handle_*` directly without
    /// spinning up wiremock — verifies the handler/state plumbing
    /// passes through an engine's results unchanged.
    struct StubEngine;
    #[async_trait]
    impl MlEngine for StubEngine {
        async fn embed(&self, _: &str) -> Result<Vec<f32>, MlError> {
            Ok(vec![0.1, 0.2, 0.3])
        }
        async fn sentiment(&self, _: &str) -> Result<Sentiment, MlError> {
            Ok(Sentiment {
                label: SentimentLabel::Positive,
                confidence: 0.91,
            })
        }
        async fn summarize(&self, _: &str, _: usize) -> Result<String, MlError> {
            Ok("ok".into())
        }
        async fn extract_entities(&self, _: &str) -> Result<Vec<Entity>, MlError> {
            Ok(vec![Entity {
                text: "Tehran".into(),
                kind: "GPE".into(),
                confidence: Some(0.9),
                start: None,
                end: None,
            }])
        }
    }

    fn state_with_stub() -> MlEngineState {
        let state = MlEngineState::new();
        // Pre-populate the cache so `engine()` skips the
        // `build_from_vault` path entirely. Production never does
        // this; tests need it to bypass the keychain dep. Drop the
        // guard before returning so the borrow doesn't outlive
        // the `state` move.
        {
            let mut guard = state.cached.try_lock().expect("test single-threaded");
            *guard = Some(Arc::new(StubEngine) as Arc<dyn MlEngine>);
        }
        state
    }

    #[tokio::test]
    async fn handle_embed_returns_vector_with_model_id() {
        let state = state_with_stub();
        let v = InMemoryVault::new();
        let resp = handle_embed(
            &state,
            &v,
            EmbedArgs {
                text: "Iran missile launch".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(resp.vector, vec![0.1, 0.2, 0.3]);
        assert!(!resp.model.is_empty());
    }

    #[tokio::test]
    async fn handle_sentiment_returns_label_and_confidence() {
        let state = state_with_stub();
        let v = InMemoryVault::new();
        let resp = handle_sentiment(
            &state,
            &v,
            SentimentArgs {
                text: "The release is great".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(resp.label, SentimentLabel::Positive);
        assert!((resp.confidence - 0.91).abs() < 1e-9);
    }

    #[tokio::test]
    async fn handle_summarize_clamps_max_tokens() {
        let state = state_with_stub();
        let v = InMemoryVault::new();
        // Caller asks for 5; clamp floor is 16. Result is unchanged
        // (stub doesn't depend on max_tokens) but the clamp should
        // not error.
        let resp = handle_summarize(
            &state,
            &v,
            SummarizeArgs {
                text: "Body".into(),
                max_tokens: Some(5),
            },
        )
        .await
        .unwrap();
        assert_eq!(resp.summary, "ok");
    }

    #[tokio::test]
    async fn handle_extract_entities_returns_entity_list() {
        let state = state_with_stub();
        let v = InMemoryVault::new();
        let resp = handle_extract_entities(
            &state,
            &v,
            ExtractEntitiesArgs {
                text: "Iran missile from Tehran".into(),
            },
        )
        .await
        .unwrap();
        assert_eq!(resp.entities.len(), 1);
        assert_eq!(resp.entities[0].text, "Tehran");
    }
}
