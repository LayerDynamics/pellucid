//! Composite engine — delegates each [`MlEngine`] method to a
//! caller-chosen backend.
//!
//! Production wiring picks an embedder (HuggingFace by default) for
//! `embed`/`batch_embed` and a chat backend (Groq by default) for
//! `sentiment`/`summarize`/`extract_entities`. The four backend
//! slots are independent — a higher-tier user can swap the
//! `summarizer` to an Anthropic-backed engine without touching the
//! others.

use std::sync::Arc;

use async_trait::async_trait;

use crate::engine::MlEngine;
use crate::types::{Entity, MlError, Sentiment};

/// Routing engine. Each slot is an `Arc<dyn MlEngine>` so the same
/// backend can be reused across multiple slots (e.g. a single Groq
/// engine for all three chat-backed methods).
#[derive(Clone)]
pub struct CompositeEngine {
    embedder: Arc<dyn MlEngine>,
    sentiment_classifier: Arc<dyn MlEngine>,
    summarizer: Arc<dyn MlEngine>,
    entity_extractor: Arc<dyn MlEngine>,
}

impl std::fmt::Debug for CompositeEngine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CompositeEngine")
            .field("embedder", &"Arc<dyn MlEngine>")
            .field("sentiment_classifier", &"Arc<dyn MlEngine>")
            .field("summarizer", &"Arc<dyn MlEngine>")
            .field("entity_extractor", &"Arc<dyn MlEngine>")
            .finish()
    }
}

impl CompositeEngine {
    /// Build with one shared embedder and one shared chat engine —
    /// the common case.
    #[must_use]
    pub fn from_embedder_and_chat(embedder: Arc<dyn MlEngine>, chat: Arc<dyn MlEngine>) -> Self {
        Self {
            embedder,
            sentiment_classifier: Arc::clone(&chat),
            summarizer: Arc::clone(&chat),
            entity_extractor: chat,
        }
    }

    /// Build with explicit per-method backends.
    #[must_use]
    pub fn new(
        embedder: Arc<dyn MlEngine>,
        sentiment_classifier: Arc<dyn MlEngine>,
        summarizer: Arc<dyn MlEngine>,
        entity_extractor: Arc<dyn MlEngine>,
    ) -> Self {
        Self {
            embedder,
            sentiment_classifier,
            summarizer,
            entity_extractor,
        }
    }

    /// Borrow the active embedder. Useful for callers that need to
    /// know the embedder's vector dimension via a downcast (each
    /// concrete engine exposes its model id).
    #[must_use]
    pub fn embedder(&self) -> &Arc<dyn MlEngine> {
        &self.embedder
    }
}

#[async_trait]
impl MlEngine for CompositeEngine {
    async fn embed(&self, text: &str) -> Result<Vec<f32>, MlError> {
        self.embedder.embed(text).await
    }

    async fn batch_embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, MlError> {
        self.embedder.batch_embed(texts).await
    }

    async fn sentiment(&self, text: &str) -> Result<Sentiment, MlError> {
        self.sentiment_classifier.sentiment(text).await
    }

    async fn summarize(&self, text: &str, max_tokens: usize) -> Result<String, MlError> {
        self.summarizer.summarize(text, max_tokens).await
    }

    async fn extract_entities(&self, text: &str) -> Result<Vec<Entity>, MlError> {
        self.entity_extractor.extract_entities(text).await
    }
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use crate::types::SentimentLabel;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// Test backend that records which methods were called and how
    /// many times — lets us prove `CompositeEngine` actually routes
    /// to the slot we configured.
    struct Recorder {
        name: &'static str,
        embed_calls: Arc<AtomicUsize>,
        sentiment_calls: Arc<AtomicUsize>,
        summarize_calls: Arc<AtomicUsize>,
        entity_calls: Arc<AtomicUsize>,
    }
    impl Recorder {
        fn new(name: &'static str) -> Arc<Self> {
            Arc::new(Self {
                name,
                embed_calls: Arc::new(AtomicUsize::new(0)),
                sentiment_calls: Arc::new(AtomicUsize::new(0)),
                summarize_calls: Arc::new(AtomicUsize::new(0)),
                entity_calls: Arc::new(AtomicUsize::new(0)),
            })
        }
    }

    #[async_trait]
    impl MlEngine for Recorder {
        async fn embed(&self, _text: &str) -> Result<Vec<f32>, MlError> {
            self.embed_calls.fetch_add(1, Ordering::SeqCst);
            Ok(vec![0.0; 8])
        }
        async fn sentiment(&self, _text: &str) -> Result<Sentiment, MlError> {
            self.sentiment_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Sentiment {
                label: SentimentLabel::Positive,
                confidence: 0.5,
            })
        }
        async fn summarize(&self, _text: &str, _max: usize) -> Result<String, MlError> {
            self.summarize_calls.fetch_add(1, Ordering::SeqCst);
            Ok(format!("summary from {}", self.name))
        }
        async fn extract_entities(&self, _text: &str) -> Result<Vec<Entity>, MlError> {
            self.entity_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Vec::new())
        }
    }

    #[tokio::test]
    async fn embed_routes_to_embedder_only() {
        let embedder = Recorder::new("embedder");
        let chat = Recorder::new("chat");
        let composite = CompositeEngine::from_embedder_and_chat(
            Arc::clone(&embedder) as Arc<dyn MlEngine>,
            Arc::clone(&chat) as Arc<dyn MlEngine>,
        );
        let _ = composite.embed("hello").await.unwrap();
        let _ = composite.batch_embed(&["a", "b"]).await.unwrap();
        // batch_embed default impl calls embed twice, so embed_calls
        // = 1 (the explicit call) + 2 (from batch) = 3.
        assert_eq!(embedder.embed_calls.load(Ordering::SeqCst), 3);
        assert_eq!(chat.embed_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn sentiment_summarize_extract_route_to_chat() {
        let embedder = Recorder::new("embedder");
        let chat = Recorder::new("chat");
        let composite = CompositeEngine::from_embedder_and_chat(
            Arc::clone(&embedder) as Arc<dyn MlEngine>,
            Arc::clone(&chat) as Arc<dyn MlEngine>,
        );
        let _ = composite.sentiment("text").await.unwrap();
        let _ = composite.summarize("text", 80).await.unwrap();
        let _ = composite.extract_entities("text").await.unwrap();

        assert_eq!(chat.sentiment_calls.load(Ordering::SeqCst), 1);
        assert_eq!(chat.summarize_calls.load(Ordering::SeqCst), 1);
        assert_eq!(chat.entity_calls.load(Ordering::SeqCst), 1);

        // Embedder must not see chat traffic.
        assert_eq!(embedder.sentiment_calls.load(Ordering::SeqCst), 0);
        assert_eq!(embedder.summarize_calls.load(Ordering::SeqCst), 0);
        assert_eq!(embedder.entity_calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn explicit_per_method_routing_uses_each_slot_independently() {
        let embedder = Recorder::new("embedder");
        let sentiment = Recorder::new("sentiment");
        let summarizer = Recorder::new("summarizer");
        let entities = Recorder::new("entities");

        let composite = CompositeEngine::new(
            Arc::clone(&embedder) as Arc<dyn MlEngine>,
            Arc::clone(&sentiment) as Arc<dyn MlEngine>,
            Arc::clone(&summarizer) as Arc<dyn MlEngine>,
            Arc::clone(&entities) as Arc<dyn MlEngine>,
        );

        let _ = composite.summarize("a", 80).await.unwrap();
        let _ = composite.extract_entities("a").await.unwrap();
        let _ = composite.sentiment("a").await.unwrap();
        let _ = composite.embed("a").await.unwrap();

        assert_eq!(embedder.embed_calls.load(Ordering::SeqCst), 1);
        assert_eq!(sentiment.sentiment_calls.load(Ordering::SeqCst), 1);
        assert_eq!(summarizer.summarize_calls.load(Ordering::SeqCst), 1);
        assert_eq!(entities.entity_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn errors_propagate_unchanged() {
        struct Failing;
        #[async_trait]
        impl MlEngine for Failing {
            async fn embed(&self, _text: &str) -> Result<Vec<f32>, MlError> {
                Err(MlError::Unsupported("embed"))
            }
            async fn sentiment(&self, _text: &str) -> Result<Sentiment, MlError> {
                Err(MlError::EmptyInput("sentiment"))
            }
            async fn summarize(&self, _text: &str, _max: usize) -> Result<String, MlError> {
                unreachable!()
            }
            async fn extract_entities(&self, _text: &str) -> Result<Vec<Entity>, MlError> {
                unreachable!()
            }
        }
        let failing: Arc<dyn MlEngine> = Arc::new(Failing);
        let composite =
            CompositeEngine::from_embedder_and_chat(Arc::clone(&failing), Arc::clone(&failing));
        assert!(matches!(
            composite.embed("x").await,
            Err(MlError::Unsupported("embed"))
        ));
        assert!(matches!(
            composite.sentiment("x").await,
            Err(MlError::EmptyInput("sentiment"))
        ));
    }
}
