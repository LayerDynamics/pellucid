//! `MlEngine` trait — the surface every backend implements.
//!
//! The trait is `async` (via [`async_trait::async_trait`]) because
//! every concrete backend in this crate is HTTP-based: HuggingFace
//! Inference API for embeddings, Groq for chat completions. There is
//! no on-device ONNX runtime in the binary; the spec's bundled-models
//! plan was replaced with a remote-API design — see the workspace
//! README + `docs/specs/SPEC-001-pellucid-stack-rebuild.md` §18 for
//! the rationale.
//!
//! Trait methods accept `&self` (not `&mut self`) so a single engine
//! can be shared across the request pipeline behind an `Arc`. Each
//! HTTP backend is internally synchronised with `reqwest::Client`'s
//! cheap-clone connection pool.

use async_trait::async_trait;

use crate::types::{Entity, MlError, Sentiment};

/// Five-method ML surface. Every method is `async` and returns a
/// [`MlError`] on transport / decode / config failure so callers can
/// distinguish "remote down" (retry) from "input invalid" (don't).
#[async_trait]
pub trait MlEngine: Send + Sync {
    /// Embed a single text into a dense vector. Vector dimension is
    /// backend-specific and stable per model — the caller is
    /// responsible for keeping the index dimension matched to the
    /// query dimension (mismatched dims will produce nonsense
    /// distances, not an error).
    async fn embed(&self, text: &str) -> Result<Vec<f32>, MlError>;

    /// Embed a batch of texts. Default implementation calls
    /// [`Self::embed`] sequentially; HTTP backends override for
    /// batched-request efficiency.
    async fn batch_embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, MlError> {
        let mut out = Vec::with_capacity(texts.len());
        for t in texts {
            out.push(self.embed(t).await?);
        }
        Ok(out)
    }

    /// Classify the sentiment of `text`.
    async fn sentiment(&self, text: &str) -> Result<Sentiment, MlError>;

    /// Produce a summary of `text` capped at approximately
    /// `max_tokens` tokens. The bound is approximate because remote
    /// backends count tokens with their own tokenizer; the prompt
    /// asks the model to stay under the cap.
    async fn summarize(&self, text: &str, max_tokens: usize) -> Result<String, MlError>;

    /// Extract named entities from `text`. Returns an empty `Vec`
    /// when the upstream finds none — never `Err` for "no entities
    /// here", only for transport / decode failures.
    async fn extract_entities(&self, text: &str) -> Result<Vec<Entity>, MlError>;
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    /// Test double that records call counts so we can verify the
    /// default `batch_embed` impl actually delegates to `embed`.
    struct CountingEngine {
        embed_calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl MlEngine for CountingEngine {
        async fn embed(&self, _text: &str) -> Result<Vec<f32>, MlError> {
            self.embed_calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![0.1, 0.2, 0.3])
        }
        async fn sentiment(&self, _text: &str) -> Result<Sentiment, MlError> {
            Err(MlError::Unsupported("sentiment"))
        }
        async fn summarize(&self, _text: &str, _max: usize) -> Result<String, MlError> {
            Err(MlError::Unsupported("summarize"))
        }
        async fn extract_entities(&self, _text: &str) -> Result<Vec<Entity>, MlError> {
            Err(MlError::Unsupported("extract_entities"))
        }
    }

    #[tokio::test]
    async fn default_batch_embed_delegates_to_embed_per_input() {
        let counter = Arc::new(AtomicUsize::new(0));
        let engine = CountingEngine {
            embed_calls: Arc::clone(&counter),
        };
        let texts = ["a", "b", "c"];
        let out = engine.batch_embed(&texts).await.unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(counter.load(Ordering::SeqCst), 3);
        for v in out {
            assert_eq!(v, vec![0.1, 0.2, 0.3]);
        }
    }

    #[tokio::test]
    async fn default_batch_embed_propagates_first_error() {
        struct Failing;
        #[async_trait]
        impl MlEngine for Failing {
            async fn embed(&self, _text: &str) -> Result<Vec<f32>, MlError> {
                Err(MlError::EmptyInput("embed"))
            }
            async fn sentiment(&self, _text: &str) -> Result<Sentiment, MlError> {
                unreachable!()
            }
            async fn summarize(&self, _text: &str, _max: usize) -> Result<String, MlError> {
                unreachable!()
            }
            async fn extract_entities(&self, _text: &str) -> Result<Vec<Entity>, MlError> {
                unreachable!()
            }
        }
        let engine = Failing;
        let res = engine.batch_embed(&["x", "y"]).await;
        assert!(matches!(res, Err(MlError::EmptyInput("embed"))));
    }
}
