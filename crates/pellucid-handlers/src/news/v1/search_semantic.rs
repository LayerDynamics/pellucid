//! `POST /api/news/v1/search-semantic` — semantic article search.
//!
//! Reads `{ query, limit? }`, embeds the query via
//! [`pellucid_ml::MlEngine::embed`], then scans the
//! `news_embeddings` table (created by migration
//! `0002_news_embeddings`) for the top-N rows by cosine similarity.
//!
//! When `sqlite-vec` lands the scan turns into a `MATCH … k = N`
//! query and this handler keeps the same wire shape; the change is
//! confined to `top_k_by_cosine` below.
//!
//! Tier-2 endpoint (`pellucid-auth::ML_ENDPOINT_ENTITLEMENTS:/api/news/v1/search-semantic`).

use std::time::Duration;

use axum::extract::State;
use axum::http::{HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use pellucid_gateway::error_mapper::GATEWAY_ERROR_CODE_HEADER;
use pellucid_ml::MlError;

use crate::state::AppState;

pub const PATH: &str = "/api/news/v1/search-semantic";
const DEFAULT_RETRY_AFTER_SECS: u32 = 30;
const DEFAULT_LIMIT: usize = 20;
const MAX_LIMIT: usize = 100;
const MIN_LIMIT: usize = 1;
const MAX_QUERY_BYTES: usize = 4 * 1024;

#[derive(Debug, Deserialize)]
pub struct Request {
    pub query: String,
    /// Number of results. Defaults to 20; clamped to `[1, 100]`.
    #[serde(default)]
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SearchHit {
    #[serde(rename = "articleId")]
    pub article_id: String,
    pub title: String,
    pub url: String,
    pub source: String,
    /// `[-1.0, 1.0]` cosine similarity. Higher = more similar.
    pub score: f64,
    /// ms epoch — same encoding the seeder writes.
    #[serde(rename = "pubDateMs")]
    pub pub_date_ms: i64,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct SearchSemanticResponse {
    pub hits: Vec<SearchHit>,
    /// Embedding model used for the query — callers can compare
    /// against indexed corpus model to detect drift.
    pub model: String,
    /// Total rows considered (post model-filter). Useful for the
    /// webview's "no matching index" empty-state copy.
    pub scanned: usize,
}

fn err_response(status: StatusCode, body: &str, retry_after: Option<u32>) -> Response {
    let body_json = serde_json::json!({
        "code": "upstream_error",
        "message": body,
    });
    let mut resp = (status, Json(body_json)).into_response();
    if let Some(secs) = retry_after {
        if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
            resp.headers_mut().insert("retry-after", v);
        }
    }
    if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
        resp.headers_mut().insert(
            GATEWAY_ERROR_CODE_HEADER,
            HeaderValue::from_static("upstream_error"),
        );
    }
    resp
}

pub async fn handler(
    State(state): State<AppState>,
    Json(req): Json<Request>,
) -> Response {
    let trimmed = req.query.trim();
    if trimmed.is_empty() {
        return err_response(StatusCode::BAD_REQUEST, "query is empty", None);
    }
    if req.query.len() > MAX_QUERY_BYTES {
        return err_response(
            StatusCode::PAYLOAD_TOO_LARGE,
            "query exceeds 4 KiB limit",
            None,
        );
    }
    let limit = req.limit.unwrap_or(DEFAULT_LIMIT).clamp(MIN_LIMIT, MAX_LIMIT);

    let Some(engine) = state.ml.as_ref() else {
        tracing::warn!(
            target: "pellucid::handlers::news",
            path = PATH,
            "MlEngine absent — returning 503"
        );
        return err_response(
            StatusCode::SERVICE_UNAVAILABLE,
            "ml engine not configured",
            Some(DEFAULT_RETRY_AFTER_SECS),
        );
    };

    // 1. Embed the query (~100-300 ms HF round trip).
    let embed_call = tokio::time::timeout(Duration::from_secs(20), engine.embed(trimmed)).await;
    let query_vec: Vec<f32> = match embed_call {
        Ok(Ok(v)) => v,
        Ok(Err(MlError::EmptyInput(_))) => {
            return err_response(StatusCode::BAD_REQUEST, "query is empty", None);
        }
        Ok(Err(MlError::MissingConfig(_))) => {
            return err_response(
                StatusCode::SERVICE_UNAVAILABLE,
                "ml engine not configured",
                Some(DEFAULT_RETRY_AFTER_SECS),
            );
        }
        Ok(Err(MlError::Upstream { status, .. })) => {
            return if status == 429 {
                err_response(
                    StatusCode::TOO_MANY_REQUESTS,
                    "ml upstream rate-limited",
                    Some(DEFAULT_RETRY_AFTER_SECS),
                )
            } else {
                err_response(
                    StatusCode::BAD_GATEWAY,
                    "ml upstream error",
                    Some(DEFAULT_RETRY_AFTER_SECS),
                )
            };
        }
        Ok(Err(other)) => {
            tracing::warn!(target: "pellucid::handlers::news", error = %other, "ml engine error");
            return err_response(
                StatusCode::BAD_GATEWAY,
                "ml engine error",
                Some(DEFAULT_RETRY_AFTER_SECS),
            );
        }
        Err(_) => {
            return err_response(
                StatusCode::GATEWAY_TIMEOUT,
                "ml upstream timed out",
                Some(DEFAULT_RETRY_AFTER_SECS),
            );
        }
    };

    if query_vec.is_empty() {
        return err_response(
            StatusCode::BAD_GATEWAY,
            "ml engine returned empty embedding",
            Some(DEFAULT_RETRY_AFTER_SECS),
        );
    }

    // 2. Look up the engine's currently-configured model id by
    // doing a tiny round-trip introspection. Since the trait
    // doesn't expose the model directly, we query the embedding's
    // dimension and cross-reference rows whose stored dim matches.
    // The `model` field on the response is what the engine
    // reported via `Vec<f32>::len()`; the search filter is by dim.
    let query_dim = query_vec.len();

    // 3. Scan the corpus.
    let scan = top_k_by_cosine(&state.pool, query_dim, &query_vec, limit).await;
    let result = match scan {
        Ok(out) => out,
        Err(e) => {
            tracing::warn!(target: "pellucid::handlers::news", error = %e, "search scan failed");
            return err_response(
                StatusCode::INTERNAL_SERVER_ERROR,
                "search index unavailable",
                Some(DEFAULT_RETRY_AFTER_SECS),
            );
        }
    };

    (
        StatusCode::OK,
        Json(SearchSemanticResponse {
            hits: result.hits,
            model: result.model.unwrap_or_default(),
            scanned: result.scanned,
        }),
    )
        .into_response()
}

/// Article-row payload accepted by [`upsert_embedding`] —
/// flattens the multi-arg form to a single struct.
#[derive(Debug, Clone)]
pub struct IndexedArticle<'a> {
    pub article_id: &'a str,
    pub title: &'a str,
    pub url: &'a str,
    pub source: &'a str,
    pub pub_date_ms: i64,
    pub model: &'a str,
    pub embedding: &'a [f32],
    pub indexed_at_ms: i64,
}

/// Index (insert-or-replace) one article into `news_embeddings`.
///
/// Used by the seeder side (the embed_articles loop) and by tests.
/// Encodes the `Vec<f32>` as little-endian f32 BLOB. Idempotent —
/// re-indexing the same `article_id` overwrites the row (article
/// titles + bodies sometimes get edited upstream; we want the
/// latest text reflected in search results).
pub async fn upsert_embedding(
    pool: &pellucid_db::Pool,
    article: IndexedArticle<'_>,
) -> Result<(), sqlx::Error> {
    if article.embedding.is_empty() {
        return Err(sqlx::Error::Protocol(
            "refusing to index empty embedding".into(),
        ));
    }
    let blob = encode_f32_le(article.embedding);
    let dim = i64::try_from(article.embedding.len()).unwrap_or(i64::MAX);
    sqlx::query(
        r"INSERT INTO news_embeddings
            (article_id, title, url, source, pub_date_ms, model, embedding, embedding_dim, indexed_at_ms)
          VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
          ON CONFLICT(article_id) DO UPDATE SET
            title          = excluded.title,
            url            = excluded.url,
            source         = excluded.source,
            pub_date_ms    = excluded.pub_date_ms,
            model          = excluded.model,
            embedding      = excluded.embedding,
            embedding_dim  = excluded.embedding_dim,
            indexed_at_ms  = excluded.indexed_at_ms",
    )
    .bind(article.article_id)
    .bind(article.title)
    .bind(article.url)
    .bind(article.source)
    .bind(article.pub_date_ms)
    .bind(article.model)
    .bind(blob)
    .bind(dim)
    .bind(article.indexed_at_ms)
    .execute(pool)
    .await?;
    Ok(())
}

/// Result of a single corpus scan — bundled into a struct rather
/// than a 3-tuple to keep clippy::type_complexity quiet.
#[derive(Debug)]
struct ScanResult {
    hits: Vec<SearchHit>,
    scanned: usize,
    model: Option<String>,
}

/// Row tuple decoded from `news_embeddings`. Tightly typed alias
/// purely to keep clippy::type_complexity happy on the
/// `query_as::<EmbeddingRow>` line.
type EmbeddingRow = (String, String, String, String, i64, String, Vec<u8>);

/// Pull every row whose stored dim matches the query, decode the
/// blobs, compute cosine similarity in-memory, return the top
/// `limit` plus the model the rows used (for the response
/// envelope).
async fn top_k_by_cosine(
    pool: &pellucid_db::Pool,
    query_dim: usize,
    query_vec: &[f32],
    limit: usize,
) -> Result<ScanResult, sqlx::Error> {
    let dim_i64 = i64::try_from(query_dim).unwrap_or(i64::MAX);
    let rows: Vec<EmbeddingRow> = sqlx::query_as(
        r"SELECT article_id, title, url, source, pub_date_ms, model, embedding
          FROM news_embeddings
          WHERE embedding_dim = ?",
    )
    .bind(dim_i64)
    .fetch_all(pool)
    .await?;

    let scanned = rows.len();
    let q_norm = l2_norm(query_vec);
    if q_norm < f64::EPSILON {
        return Ok(ScanResult {
            hits: Vec::new(),
            scanned,
            model: None,
        });
    }

    let mut model_seen: Option<String> = None;
    let mut scored: Vec<(f64, SearchHit)> = Vec::with_capacity(scanned);
    for (article_id, title, url, source, pub_date_ms, model, blob) in rows {
        if model_seen.is_none() {
            model_seen = Some(model.clone());
        }
        let v = decode_f32_le(&blob, query_dim);
        let Some(v) = v else { continue };
        let v_norm = l2_norm(&v);
        if v_norm < f64::EPSILON {
            continue;
        }
        let dot = dot_product(query_vec, &v);
        let score = dot / (q_norm * v_norm);
        scored.push((
            score,
            SearchHit {
                article_id,
                title,
                url,
                source,
                score,
                pub_date_ms,
            },
        ));
    }
    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.1.pub_date_ms.cmp(&a.1.pub_date_ms))
    });
    let hits: Vec<SearchHit> = scored.into_iter().take(limit).map(|(_, h)| h).collect();
    Ok(ScanResult {
        hits,
        scanned,
        model: model_seen,
    })
}

fn encode_f32_le(v: &[f32]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(v.len() * 4);
    for f in v {
        buf.extend_from_slice(&f.to_le_bytes());
    }
    buf
}

fn decode_f32_le(blob: &[u8], expected_dim: usize) -> Option<Vec<f32>> {
    if blob.len() != expected_dim * 4 {
        return None;
    }
    let mut v = Vec::with_capacity(expected_dim);
    for chunk in blob.chunks_exact(4) {
        let arr: [u8; 4] = chunk.try_into().ok()?;
        v.push(f32::from_le_bytes(arr));
    }
    Some(v)
}

fn dot_product(a: &[f32], b: &[f32]) -> f64 {
    let mut s = 0.0_f64;
    for (x, y) in a.iter().zip(b.iter()) {
        s += f64::from(*x) * f64::from(*y);
    }
    s
}

fn l2_norm(v: &[f32]) -> f64 {
    let mut s = 0.0_f64;
    for x in v {
        s += f64::from(*x) * f64::from(*x);
    }
    s.sqrt()
}

#[cfg(test)]
#[allow(clippy::panic, clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use axum::body::{to_bytes, Body};
    use axum::http::Request as HttpRequest;
    use std::sync::Arc;
    use tower::ServiceExt;

    use pellucid_ml::{Entity, MlEngine, Sentiment};

    /// Stub engine that returns a fixed query vector. Tests pin the
    /// vector so cosine-sim ordering is deterministic.
    struct FixedEmbedder {
        vec: Vec<f32>,
    }
    #[async_trait]
    impl MlEngine for FixedEmbedder {
        async fn embed(&self, _: &str) -> Result<Vec<f32>, MlError> {
            Ok(self.vec.clone())
        }
        async fn sentiment(&self, _: &str) -> Result<Sentiment, MlError> {
            Err(MlError::Unsupported("sentiment"))
        }
        async fn summarize(&self, _: &str, _: usize) -> Result<String, MlError> {
            Err(MlError::Unsupported("summarize"))
        }
        async fn extract_entities(&self, _: &str) -> Result<Vec<Entity>, MlError> {
            Err(MlError::Unsupported("extract_entities"))
        }
    }

    fn router(state: AppState) -> axum::Router {
        axum::Router::new().route(PATH, axum::routing::post(handler).with_state(state))
    }
    fn post(body: serde_json::Value) -> HttpRequest<Body> {
        HttpRequest::builder()
            .method("POST")
            .uri(PATH)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn returns_503_when_ml_engine_absent() {
        let state = AppState::for_tests_async().await.unwrap();
        let app = router(state);
        let resp = app
            .oneshot(post(serde_json::json!({"query": "iran missile"})))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn returns_400_for_empty_query() {
        let state = AppState::for_tests_async()
            .await
            .unwrap()
            .with_ml(Arc::new(FixedEmbedder {
                vec: vec![1.0, 0.0, 0.0],
            }));
        let app = router(state);
        let resp = app
            .oneshot(post(serde_json::json!({"query": "  "})))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn returns_413_for_oversized_query() {
        let state = AppState::for_tests_async()
            .await
            .unwrap()
            .with_ml(Arc::new(FixedEmbedder {
                vec: vec![1.0, 0.0, 0.0],
            }));
        let app = router(state);
        let huge = "x".repeat(MAX_QUERY_BYTES + 1);
        let resp = app
            .oneshot(post(serde_json::json!({"query": huge})))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn returns_200_with_top_k_cosine_match_when_corpus_indexed() {
        // Index 3 articles with 3-d embeddings. Query is aligned
        // with article B → B should be top-1; ranking by cosine.
        let state = AppState::for_tests_async()
            .await
            .unwrap()
            .with_ml(Arc::new(FixedEmbedder {
                vec: vec![1.0, 0.0, 0.0],
            }));
        // Article A: opposite-direction vector → low score.
        upsert_embedding(
            &state.pool,
            IndexedArticle {
                article_id: "a",
                title: "Apple earnings beat",
                url: "https://example/a",
                source: "Reuters",
                pub_date_ms: 10,
                model: "test-3d",
                embedding: &[-1.0, 0.0, 0.0],
                indexed_at_ms: 100,
            },
        )
        .await
        .unwrap();
        // Article B: aligned with query → cosine = 1.0.
        upsert_embedding(
            &state.pool,
            IndexedArticle {
                article_id: "b",
                title: "Iran missile launch",
                url: "https://example/b",
                source: "AP",
                pub_date_ms: 20,
                model: "test-3d",
                embedding: &[1.0, 0.0, 0.0],
                indexed_at_ms: 100,
            },
        )
        .await
        .unwrap();
        // Article C: orthogonal → cosine = 0.
        upsert_embedding(
            &state.pool,
            IndexedArticle {
                article_id: "c",
                title: "Stock market rally",
                url: "https://example/c",
                source: "CNBC",
                pub_date_ms: 30,
                model: "test-3d",
                embedding: &[0.0, 1.0, 0.0],
                indexed_at_ms: 100,
            },
        )
        .await
        .unwrap();

        let app = router(state);
        let resp = app
            .oneshot(post(serde_json::json!({"query": "Iran strike"})))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: SearchSemanticResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.scanned, 3);
        assert_eq!(body.model, "test-3d");
        assert_eq!(body.hits.len(), 3);
        // Top hit is B (cos = 1.0).
        assert_eq!(body.hits[0].article_id, "b");
        assert!((body.hits[0].score - 1.0).abs() < 1e-6);
        // Last hit is A (cos = -1.0).
        assert_eq!(body.hits[2].article_id, "a");
        assert!(body.hits[2].score < 0.0);
    }

    #[tokio::test]
    async fn limit_clamped_to_caps() {
        let state = AppState::for_tests_async()
            .await
            .unwrap()
            .with_ml(Arc::new(FixedEmbedder {
                vec: vec![1.0, 0.0],
            }));
        for i in 0..5 {
            let id = format!("doc{i}");
            upsert_embedding(
                &state.pool,
                IndexedArticle {
                    article_id: &id,
                    title: "t",
                    url: "u",
                    source: "s",
                    pub_date_ms: 0,
                    model: "test-2d",
                    embedding: &[1.0, 0.0],
                    indexed_at_ms: 0,
                },
            )
            .await
            .unwrap();
        }
        let app = router(state);
        // Caller asks for 0 → clamped up to 1. Returns 1 row.
        let resp = app
            .clone()
            .oneshot(post(serde_json::json!({"query": "x", "limit": 0})))
            .await
            .unwrap();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: SearchSemanticResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.hits.len(), 1);
        // Caller asks for 9999 → clamped to 100. Corpus is only 5
        // → 5 rows returned.
        let resp = app
            .oneshot(post(serde_json::json!({"query": "x", "limit": 9999})))
            .await
            .unwrap();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: SearchSemanticResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.hits.len(), 5);
    }

    #[tokio::test]
    async fn returns_empty_hits_when_no_matching_dim() {
        // Indexed corpus is 2-d; query embedder returns 3-d → no
        // rows match the dim filter. Should be 200 with empty hits
        // (not an error — the webview's empty state handles it).
        let state = AppState::for_tests_async()
            .await
            .unwrap()
            .with_ml(Arc::new(FixedEmbedder {
                vec: vec![1.0, 0.0, 0.0],
            }));
        upsert_embedding(
            &state.pool,
            IndexedArticle {
                article_id: "a",
                title: "t",
                url: "u",
                source: "s",
                pub_date_ms: 0,
                model: "test-2d",
                embedding: &[1.0, 0.0],
                indexed_at_ms: 0,
            },
        )
        .await
        .unwrap();
        let app = router(state);
        let resp = app
            .oneshot(post(serde_json::json!({"query": "x"})))
            .await
            .unwrap();
        let bytes = to_bytes(resp.into_body(), 1 << 20).await.unwrap();
        let body: SearchSemanticResponse = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body.scanned, 0);
        assert!(body.hits.is_empty());
    }

    #[tokio::test]
    async fn upsert_replaces_existing_row() {
        let state = AppState::for_tests_async().await.unwrap();
        upsert_embedding(
            &state.pool,
            IndexedArticle {
                article_id: "x",
                title: "first",
                url: "u1",
                source: "s",
                pub_date_ms: 0,
                model: "test-2d",
                embedding: &[1.0, 0.0],
                indexed_at_ms: 0,
            },
        )
        .await
        .unwrap();
        upsert_embedding(
            &state.pool,
            IndexedArticle {
                article_id: "x",
                title: "second",
                url: "u2",
                source: "s",
                pub_date_ms: 0,
                model: "test-2d",
                embedding: &[0.0, 1.0],
                indexed_at_ms: 0,
            },
        )
        .await
        .unwrap();
        let row: (String,) =
            sqlx::query_as("SELECT title FROM news_embeddings WHERE article_id = ?")
                .bind("x")
                .fetch_one(&state.pool)
                .await
                .unwrap();
        assert_eq!(row.0, "second");
    }

    #[test]
    fn encode_decode_round_trips_le_f32() {
        let v = vec![0.1_f32, -0.2, 0.3, 1e-9, -1e9];
        let blob = encode_f32_le(&v);
        let back = decode_f32_le(&blob, v.len()).unwrap();
        assert_eq!(v.len(), back.len());
        for (a, b) in v.iter().zip(back.iter()) {
            assert!((a - b).abs() < 1e-30, "{a} vs {b}");
        }
    }

    #[test]
    fn decode_rejects_blob_with_wrong_length() {
        assert!(decode_f32_le(&[0u8; 9], 2).is_none());
        assert!(decode_f32_le(&[0u8; 8], 2).is_some());
    }

    #[test]
    fn cosine_components_match_textbook() {
        let a = [1.0_f32, 0.0, 0.0];
        let b = [1.0, 0.0, 0.0];
        let c = [0.0, 1.0, 0.0];
        let d = [-1.0, 0.0, 0.0];
        assert!((dot_product(&a, &b) / (l2_norm(&a) * l2_norm(&b)) - 1.0).abs() < 1e-9);
        assert!(dot_product(&a, &c).abs() < 1e-9);
        assert!((dot_product(&a, &d) / (l2_norm(&a) * l2_norm(&d)) + 1.0).abs() < 1e-9);
    }
}
