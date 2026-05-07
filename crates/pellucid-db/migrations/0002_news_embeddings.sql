-- 0002_news_embeddings.sql — semantic-search index for news articles.
--
-- SPEC-001 §18.4 specifies a `sqlite-vec` virtual table for the
-- query path. We're not loading sqlite-vec into the binary today
-- (extension load complexity per platform); instead the handler at
-- `pellucid_handlers::news::v1::search_semantic` does in-memory
-- cosine similarity over the rows in this table. With ~thousands of
-- articles this is sub-50 ms in release builds. When sqlite-vec
-- lands, the migration replaces the BLOB column with a virtual
-- table backed by `vec0(embedding float[D])` and the handler
-- switches to `MATCH … k = N`; the public RPC contract doesn't
-- change.

CREATE TABLE IF NOT EXISTS news_embeddings (
    -- Stable canonical identity for the article. Whatever the
    -- caller used as the cluster id / RSS guid; this is the row
    -- the search handler returns.
    article_id      TEXT PRIMARY KEY,
    -- Display title — denormalised so search responses don't have
    -- to JOIN against the articles cache.
    title           TEXT NOT NULL,
    -- Original article URL (or empty when not available). Used by
    -- the handler so the webview can render a "open" link.
    url             TEXT NOT NULL DEFAULT '',
    -- Source feed name (e.g. "Reuters"). Empty when unknown.
    source          TEXT NOT NULL DEFAULT '',
    -- ISO-8601 publication timestamp. Used for recency tie-break +
    -- freshness filtering. Stored as ms epoch for cheap ordering.
    pub_date_ms     INTEGER NOT NULL DEFAULT 0,
    -- Embedding model id (e.g. `sentence-transformers/all-MiniLM-L6-v2`).
    -- The handler refuses to query rows whose `model` differs
    -- from the engine's currently-configured model — mismatched
    -- dimensions yield nonsense distances.
    model           TEXT NOT NULL,
    -- Embedding vector, encoded as little-endian f32 bytes
    -- (4 × dim). The handler decodes back to `Vec<f32>` for
    -- cosine sim. Stored as BLOB rather than JSON to keep
    -- per-row size compact (1.5 KiB for a 384-dim MiniLM
    -- vector, vs ~4 KiB JSON).
    embedding       BLOB NOT NULL,
    -- Vector dimension. Stored explicitly so the handler can
    -- detect dimension drift without decoding the blob.
    embedding_dim   INTEGER NOT NULL,
    -- ms epoch of the last time this row was written.
    indexed_at_ms   INTEGER NOT NULL
) WITHOUT ROWID;

-- Most queries filter by model first (single model per deploy
-- after a re-index) so a covering index on (model, pub_date_ms)
-- speeds the recency-bounded scans the handler does when a
-- `since` filter is supplied.
CREATE INDEX IF NOT EXISTS news_embeddings_model_pubdate
    ON news_embeddings (model, pub_date_ms);

CREATE INDEX IF NOT EXISTS news_embeddings_indexed_at
    ON news_embeddings (indexed_at_ms);
