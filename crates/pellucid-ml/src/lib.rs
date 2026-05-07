//! pellucid-ml — ML engine surface for the Pellucid stack.
//!
//! ## Design
//!
//! Three concrete backends, all HTTP-based:
//!
//! - [`HfEmbeddingEngine`] — HuggingFace Inference API for `embed`
//!   and `batch_embed`. Default model
//!   `sentence-transformers/all-MiniLM-L6-v2` (384-dim, free-tier).
//! - [`GroqEngine`] — Groq chat completions for `sentiment`,
//!   `summarize`, and `extract_entities`. Default model
//!   `llama-3.1-8b-instant`. Embeddings are
//!   [`MlError::Unsupported`].
//! - [`CompositeEngine`] — routes per-method to a caller-chosen
//!   backend. Production wiring uses one HF embedder + one Groq
//!   chat engine.
//!
//! ## Why no on-device ONNX?
//!
//! The original spec (`docs/specs/SPEC-001-pellucid-stack-rebuild.md`
//! §18) called for `ort` 2.x with four bundled ONNX models. After
//! reading the WorldMonitor source we found:
//!
//! - Bundling MiniLM-L6 + DistilBERT-SST2 + Flan-T5 + BERT-NER ships
//!   ~150 MB of binary weight per platform.
//! - `summarize` and `extract_entities` quality on tiny ONNX models
//!   is markedly worse than Llama-3.1-8B prompted via Groq's free
//!   tier — and Groq is *faster* end-to-end on cold cache because
//!   no model load step.
//! - Embeddings still need to live in the same vector space across
//!   the seeder pipeline and query path; an HF Inference API call
//!   per query (~200-500 ms) is acceptable for semantic search and
//!   removes the ONNX runtime from the binary entirely.
//!
//! ## Wiring
//!
//! API keys come from a [`pellucid_core::vault::Vault`] in the
//! desktop host (`KeychainVault`) and from environment variables on
//! `pellucid-edge-bin` (`EnvVault`). Engines fail closed with
//! [`MlError::MissingConfig`] when keys are absent so the binary
//! returns 503 rather than serving fixture data.
//!
//! ```ignore
//! use std::sync::Arc;
//! use pellucid_ml::{
//!     CompositeEngine, GroqEngine, HfEmbeddingEngine, MlEngine,
//! };
//!
//! # async fn boot(hf_token: String, groq_key: String) -> anyhow::Result<()> {
//! let embedder = Arc::new(HfEmbeddingEngine::new(hf_token)?);
//! let chat = Arc::new(GroqEngine::new(groq_key)?);
//! let engine: Arc<dyn MlEngine> = Arc::new(
//!     CompositeEngine::from_embedder_and_chat(embedder, chat),
//! );
//! let v = engine.embed("a news headline").await?;
//! assert_eq!(v.len(), 384);
//! # Ok(())
//! # }
//! ```

pub mod composite;
pub mod engine;
pub mod groq;
pub mod huggingface;
pub mod types;

pub use composite::CompositeEngine;
pub use engine::MlEngine;
pub use groq::{GroqEngine, GroqEngineBuilder, DEFAULT_MODEL as GROQ_DEFAULT_MODEL};
pub use huggingface::{
    HfEmbeddingEngine, HfEmbeddingEngineBuilder, DEFAULT_MODEL as HF_DEFAULT_EMBEDDING_MODEL,
};
pub use types::{Entity, MlError, Sentiment, SentimentLabel};

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

    #[test]
    fn default_model_constants_are_non_empty() {
        assert!(!GROQ_DEFAULT_MODEL.is_empty());
        assert!(!HF_DEFAULT_EMBEDDING_MODEL.is_empty());
    }
}
