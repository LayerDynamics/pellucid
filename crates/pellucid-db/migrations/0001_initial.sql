-- 0001_initial.sql — Pellucid canonical schema (SPEC-001 §6).
--
-- Every table mandated by SPEC-001 §6.1 and every virtual table from
-- §6.2 is created here. Re-running this migration against an existing
-- database is a no-op because every CREATE uses IF NOT EXISTS.

------------------------------------------------------------------------
-- §6.1 Core tables
------------------------------------------------------------------------

-- Cache envelope: replaces Redis SET/GET of envelope payloads.
CREATE TABLE IF NOT EXISTS kv_envelope (
    cache_key       TEXT PRIMARY KEY,
    payload         TEXT NOT NULL,                      -- JSON; {_seed,data} when enveloped
    fetched_at_ms   INTEGER NOT NULL,
    ttl_ms          INTEGER NOT NULL,
    record_count    INTEGER,
    source_version  TEXT,
    state           TEXT,                               -- 'live'|'stale'|'backup'|'seeded'
    is_negative     INTEGER NOT NULL DEFAULT 0          -- negative-cache sentinel
) WITHOUT ROWID;

CREATE INDEX IF NOT EXISTS kv_envelope_fetched
    ON kv_envelope (fetched_at_ms);

-- Seed metadata: replaces Redis seed-meta:* keys. Surfaced to /api/health
-- so the cascade classifier can distinguish 'missing' from 'stale'.
CREATE TABLE IF NOT EXISTS seed_meta (
    cache_key       TEXT PRIMARY KEY,
    fetched_at_ms   INTEGER NOT NULL,
    ttl_ms          INTEGER NOT NULL,
    last_run_id     TEXT NOT NULL,
    source_version  TEXT,
    record_count    INTEGER,
    cascade_group   TEXT
) WITHOUT ROWID;

-- Distributed locks: replaces Redis SET NX PX seed-lock:*. atomic_publish
-- uses BEGIN IMMEDIATE so SQLite serializes writers; the row gives us
-- crash-recovery info (run_id) for the cleanup pass.
CREATE TABLE IF NOT EXISTS seed_lock (
    domain          TEXT PRIMARY KEY,
    run_id          TEXT NOT NULL,
    expires_at_ms   INTEGER NOT NULL
) WITHOUT ROWID;

-- Entitlement cache: replaces entitlements:${ENV_PREFIX}:${userId} 15min
-- TTL key in Redis. Mirrored from Convex by pellucid-auth on cache miss.
CREATE TABLE IF NOT EXISTS entitlements_cache (
    user_id         TEXT PRIMARY KEY,
    tier            INTEGER NOT NULL,
    features_json   TEXT NOT NULL,
    valid_until_ms  INTEGER NOT NULL,
    cached_at_ms    INTEGER NOT NULL
) WITHOUT ROWID;

-- Rate limit windows: replaces Upstash sliding window. Aggregate (rl:agg)
-- bucket also lives here per M8 fix (SPEC-001 §24.3 M8).
CREATE TABLE IF NOT EXISTS rate_limit_window (
    bucket_key      TEXT NOT NULL,
    request_at_ms   INTEGER NOT NULL,
    PRIMARY KEY (bucket_key, request_at_ms)
);

CREATE INDEX IF NOT EXISTS rate_limit_lookup
    ON rate_limit_window (bucket_key, request_at_ms);

-- User-side preferences (panel layout). Kept here for desktop; the
-- web build mirrors via convex when entitlements snapshot lives there.
CREATE TABLE IF NOT EXISTS panel_layout (
    user_id         TEXT NOT NULL,
    panel_id        TEXT NOT NULL,
    variant         TEXT NOT NULL,
    layout_json     TEXT NOT NULL,
    updated_at_ms   INTEGER NOT NULL,
    PRIMARY KEY (user_id, panel_id, variant)
) WITHOUT ROWID;

-- Webhook idempotency (Dodo). OP-18.
CREATE TABLE IF NOT EXISTS webhook_seen (
    webhook_id      TEXT PRIMARY KEY,
    received_at_ms  INTEGER NOT NULL,
    payload_hash    TEXT NOT NULL
) WITHOUT ROWID;

-- Internal version tracker so future migrations can be idempotent + ordered.
CREATE TABLE IF NOT EXISTS pellucid_migrations (
    version         INTEGER PRIMARY KEY,
    name            TEXT NOT NULL,
    applied_at_ms   INTEGER NOT NULL
);

INSERT OR IGNORE INTO pellucid_migrations (version, name, applied_at_ms)
VALUES (1, '0001_initial', strftime('%s', 'now') * 1000);

------------------------------------------------------------------------
-- §6.2 Specialized virtual tables
------------------------------------------------------------------------

-- News + intel full-text search (FTS5).
CREATE VIRTUAL TABLE IF NOT EXISTS news_fts USING fts5(
    article_id UNINDEXED,
    title,
    summary,
    body,
    source,
    lang,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Vessel + aircraft positions for spatial queries (replaces Redis geoSearchByBox).
CREATE VIRTUAL TABLE IF NOT EXISTS positions_rtree USING rtree(
    id,
    min_lon, max_lon,
    min_lat, max_lat
);

CREATE TABLE IF NOT EXISTS positions_meta (
    id              INTEGER PRIMARY KEY,
    kind            TEXT NOT NULL,                       -- 'vessel'|'aircraft'|'satellite'
    external_id     TEXT NOT NULL,
    payload_json    TEXT NOT NULL,
    observed_at_ms  INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS positions_meta_external
    ON positions_meta (kind, external_id);

CREATE INDEX IF NOT EXISTS positions_meta_observed
    ON positions_meta (observed_at_ms);

-- Embedding metadata. The vector column itself lives in the sqlite-vec
-- virtual table created at runtime by Pool::open() iff sqlite-vec is
-- loadable in this build (see crates/pellucid-db/src/pool.rs).
CREATE TABLE IF NOT EXISTS embedding_meta (
    rowid           INTEGER PRIMARY KEY,
    article_id      TEXT NOT NULL,
    model           TEXT NOT NULL,
    created_at_ms   INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS embedding_meta_article
    ON embedding_meta (article_id);
