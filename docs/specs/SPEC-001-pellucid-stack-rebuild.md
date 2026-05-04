# SPEC-001 — Pellucid: Stack Rebuild of WorldMonitor

**Status:** Draft for execution
**Author:** /lore:project-spec-writer
**Date:** 2026-04-25
**Source of truth:** `docs/LoreDeepCodeReview.md`, `docs/LoreWorldMonitorComponents.md` (both verified at commit HEAD on 2026-04-17 of the WorldMonitor `main` branch)
**Target outcome:** Functional parity with WorldMonitor. New stack: Tauri + Bun + Vite + Tailwind + Radix + Zustand + SQLite + Rust.

---

## 1. Product Identity (Lead With The Product)

**Pellucid is a real-time situational-awareness console.** It is an 86-panel intelligence grid covering markets, geopolitics, military posture, conflict, climate, energy, supply chain, infrastructure, cyber, and forecasting — surfaced through a deck.gl 2D map and a globe.gl 3D map, with cross-source correlation and on-device ML summarization. It ships as both a Tauri desktop app (offline-capable, locally cached) and a hosted SaaS (`worldmonitor.app` web SPA + `api.worldmonitor.app` public RPC API), with five domain-skinned variants (`base`, `tech`, `finance`, `commodity`, `happy`) selected by build flag, hostname, or user preference.

The product's marquee behavior is **continuous low-latency multi-domain hydration with stampede-protected caching, viewport-conditional refresh, and entitlement-gated premium tiers**. Pellucid is not a dashboard builder — it is a curated intelligence broadcast where the cache, the seeders, and the gateway pipeline are as important as the UI.

The rebuild preserves every product behavior described in the source documents. It changes only the stack and corrects the inherited Critical/High defects (§24).

---

## 2. Outcome-Preservation Mandate

These behaviors **must** be present at v1 GA and verifiable against the corresponding source-document citation:

| # | Preserved Behavior | Source citation |
|---|---|---|
| OP-1 | All 86 panels and their families ship | `LoreWorldMonitorComponents.md §3` |
| OP-2 | 8-phase boot sequence (storage+i18n+ML init, two-tier bootstrap, Clerk auth, panel layout, search/intel, parallel data load, smart-poll loop, desktop updater) | `LoreDeepCodeReview.md §1.2 Path A` |
| OP-3 | 14-stage gateway pipeline at parity (origin → CORS → preflight → tier → Clerk → API key → entitlement → endpoint RL → global RL → router → handler → headers → ETag → cache headers) | `LoreDeepCodeReview.md §1.2 Path B` |
| OP-4 | Two-tier bootstrap (`/api/bootstrap?tier=fast` 3s + `?tier=slow` 5s, separate AbortControllers, 67 fast keys + 45 slow keys = 112 total) | `LoreDeepCodeReview.md §1.2 Path C` |
| OP-5 | Health-check classifier with cascade tolerance (OK / OK_CASCADE / STALE_SEED / SEED_ERROR / EMPTY / EMPTY_ON_DEMAND / REDIS_PARTIAL → roll-up HEALTHY / WARNING / DEGRADED / UNHEALTHY) | `LoreDeepCodeReview.md §1.2 Path D` |
| OP-6 | Stream relay (AIS WebSocket, OpenSky OAuth2, RSS proxy, Telegram MTProto, OREF curl-with-JA3-bypass), seed loop scheduler with ~20 cadences | `LoreDeepCodeReview.md §1.2 Path E` |
| OP-7 | Desktop sidecar with dynamic-port discovery, IPC token, IPv4-forced fetch, cloud fallback for forced paths | `LoreDeepCodeReview.md §1.2 Path F` |
| OP-8 | 6-tier cache headers (fast / medium / slow / slow-browser / static / daily / no-store) with `s-maxage`, SWR, and SIE values exactly as specified | `LoreDeepCodeReview.md §1.4` |
| OP-9 | Atomic seed publish (lock → validate → envelope → 5 MB size check → staging key → canonical key → seed-meta → release lock via Lua compare-and-del) | `LoreDeepCodeReview.md §1.4` |
| OP-10 | Clerk JWT verification with JWKS, entitlement Redis cache (15 min TTL), Convex fallback, tier map (4 endpoints currently tier-2) | `LoreDeepCodeReview.md §1.5` |
| OP-11 | Plan catalog: free(0) → pro(1) → api_starter/api_business(2) → enterprise(3); features `{tier, maxDashboards, apiAccess, apiRateLimit, prioritySupport, exportFormats}` | `LoreDeepCodeReview.md §1.5` |
| OP-12 | HMAC-signed userId in Dodo checkout metadata (`wm_user_id_sig`) verified by webhook | `LoreDeepCodeReview.md §1.5` |
| OP-13 | Variant detection chain: build-time `VITE_VARIANT` → hostname prefix → desktop localStorage; on-change reset of mapLayers, disable cross-variant panels, seed defaults | `LoreDeepCodeReview.md §1.6` |
| OP-14 | Stampede-protected cache with in-flight Map coalescing, negative-cache sentinel, opt-in envelope `{_seed:{fetchedAt,recordCount,sourceVersion,state,...}, data}` | `LoreDeepCodeReview.md §1.4` + Strength #3 |
| OP-15 | Two map renderers (deck.gl 2D with Scatterplot/GeoJson/Path/Icon/Polygon/Arc/Heatmap/H3Hexagon, PMTiles, supercluster; globe.gl 3D with merged htmlElementsData via `_kind`, atmosphere shader, auto-rotate) | `LoreWorldMonitorComponents.md §3` |
| OP-16 | Workers: ML inference (embeddings + sentiment + summarization + NER), analysis (Jaccard clustering + cross-domain correlation), vector DB (semantic search) | `LoreWorldMonitorComponents.md §5` |
| OP-17 | Smart poll loop: pauseWhenHidden, maxBackoffMultiplier:4, staggered flush (100 ms first 4, 300 ms rest) on tab visibility | `LoreDeepCodeReview.md §1.2 P7` |
| OP-18 | Webhook idempotency keyed by `webhook-id`, failed mutations return 500 so Dodo retries | `LoreDeepCodeReview.md` Strength #6 |
| OP-19 | Pre-push hook substantive: typecheck, CJS syntax, edge bundle esbuild, edge import guardrail, markdown+MDX lint, version sync | `LoreDeepCodeReview.md` Strength #9 |
| OP-20 | Proto contract enforcement in CI (sebuf + buf, generated stubs cannot drift) | `LoreDeepCodeReview.md` Strength #12 |
| OP-21 | Per-RPC env override `CACHE_TIER_OVERRIDE_${RPC_NAME}` for ops escalation without redeploy | `LoreDeepCodeReview.md` Strength #13 |
| OP-22 | Client-side circuit breakers in `data-loader` to prevent cascade on flapping upstream — ported to `webview/src/utils/circuit-breaker.ts` and consumed by `webview/src/data/loader.ts` per-domain loaders | `LoreDeepCodeReview.md` Strength #11 |
| OP-23 | Vault consolidation: single keychain entry, single Touch ID prompt, transactional migration from legacy individual entries | `LoreDeepCodeReview.md` Strength #10 |

A v1 release that does not satisfy each row (or explicitly defers it to v1.x with an owner and date) is **not** a parity rebuild and must not be claimed as one.

---

## 3. Stack Mandate & Component Mapping

### 3.1 Stack mandate (locked by user)

| Layer | Technology |
|---|---|
| Desktop shell | Tauri 2.x |
| Webview UI | React 19 + Vite + TypeScript (Radix Primitives requires a React-compatible host) |
| Styling | Tailwind CSS 4 |
| UI primitives | Radix Primitives + Radix Colors (`@radix-ui/react-*`) |
| State | Zustand 5 (with `persist` for layout, `subscribeWithSelector` for selectors) |
| JS runtime / package manager / test runner / scripts | Bun 1.x |
| Bundler | Vite 6 (+ `@tauri-apps/plugin-vite`) |
| Local + edge canonical store | SQLite 3.46+ (with `JSON1`, `FTS5`, `R*Tree`, `sqlite-vec`, `sqlite-zstd` extensions) |
| Non-public-facing systems | Rust (Cargo workspace, Tokio, Axum, Reqwest, Rusqlite/SQLx, ort/candle for ML) |
| Public-facing API (rebuilt, retained) | Rust binary on **Railway** (or self-host) — Axum + Tower |
| Hosted web SPA at `worldmonitor.app` | Same Vite-built React bundle as Tauri webview, served from Railway/Cloudflare Pages with CDN |
| Auth | Clerk (retained external SaaS) |
| Payments | Dodo (retained external SaaS) |
| Plan/entitlement DB + webhook receiver | Convex (retained — pragmatic; owns billing plumbing) |

### 3.2 Public-facing vs non-public-facing boundary (resolved)

| Surface | Public-facing? | Implementation |
|---|---|---|
| Tauri webview UI | Yes | TypeScript / React |
| Hosted web SPA `worldmonitor.app` | Yes | Same TS/React bundle |
| Public RPC API `api.worldmonitor.app` | Yes (third-party consumers) | **Rust** (Axum) — re-exposes the same RPC contract |
| Tauri desktop sidecar (in-process) | No (only the local UI calls it) | **Rust** (Axum on `127.0.0.1:dyn-port`) |
| Stream relay (AIS, OpenSky, RSS, Telegram, OREF) | No | **Rust** binary (`pellucid-relay-bin`) |
| Seed loop scheduler (~140 seeders, ~20 cadences) | No | **Rust** (modules of `pellucid-seeders`, run by sidecar and edge) |
| Gateway pipeline (14 stages) | No (internal lib used by both edge and sidecar) | **Rust** crate `pellucid-gateway` |
| Cache layer | No | **Rust** crate `pellucid-cache` over SQLite |
| ML inference | No | **Rust** crate `pellucid-ml` (ort or candle) |
| Correlation engine | No | **Rust** crate `pellucid-correlation` |
| Convex schema + webhook actions | Public surface to Dodo, but managed | Retained Convex TS |

### 3.3 Component mapping (old → new)

| Original (WorldMonitor) | Pellucid Replacement |
|---|---|
| `src/main.ts` (677 LOC) | `webview/src/main.tsx` (Vite entry, Sentry init, Vercel analytics replaced by self-hosted Plausible if desired, fetch patch installed via Tauri IPC sniffer) |
| `src/App.ts` (1486 LOC, 8-phase init) | `webview/src/app/boot.ts` — same 8 phases, orchestrated as Zustand actions; `useBootStore` state machine |
| `src/app/app-context.ts` | `webview/src/state/` — split into Zustand stores: `useMapStore`, `usePanelStore`, `useDataStore`, `useAuthStore`, `useVariantStore`, `useUiStore` |
| `src/app/data-loader.ts` (3356 LOC) | `webview/src/data/loader.ts` (UI-side) + `crates/pellucid-handlers` (server-side handlers it calls). Per-domain loaders preserved 1:1 |
| `src/app/refresh-scheduler.ts` | `webview/src/data/refresh-scheduler.ts` — `RefreshScheduler` class kept, integrated with Zustand subscription |
| `src/app/panel-layout.ts` (2142 LOC) | `webview/src/panels/PanelGrid.tsx` + `crates/pellucid-core/src/layout.rs` for serialization. Layout persisted via Tauri `store` plugin (desktop) or IndexedDB (web) |
| `src/components/Panel.ts` (1203 LOC) | `webview/src/panels/Panel.tsx` — base Radix-styled component; subclasses become composition (`<Panel><MyContent/></Panel>`) |
| 86 panel subclasses | 86 React components under `webview/src/panels/<family>/` |
| `src/components/DeckGLMap.ts` (6607 LOC) | `webview/src/maps/DeckGLMap.tsx` — same deck.gl + maplibre-gl + PMTiles + supercluster, wrapped as React component |
| `src/components/GlobeMap.ts` (3578 LOC) | `webview/src/maps/GlobeMap.tsx` — same globe.gl with `_kind` discriminator |
| `src/services/runtime.ts` (910 LOC) | `webview/src/services/runtime.ts` — Tauri IPC port discovery via `@tauri-apps/api/core invoke('get_local_api_port')`; URL builders, VisibilityHub, smart-poll loop, fetch patches preserved |
| `src/services/*` (~200 service files) | `webview/src/services/*` — pure functions consumed by panels and loaders. Heavy ones (correlation engine, clustering, entity-extraction, ml-worker) move into Rust and are exposed via IPC + RPC |
| `src/workers/analysis.worker.ts` | `crates/pellucid-correlation` (Rust) + thin webview wrapper |
| `src/workers/ml.worker.ts` (transformers.js) | `crates/pellucid-ml` (Rust, ort or candle) — embeddings, sentiment, summarization, NER. ONNX models bundled in Tauri resources |
| `src/workers/vector-db.ts` (IndexedDB) | `crates/pellucid-db` `embeddings` table with `sqlite-vec` extension |
| `src-tauri/src/main.rs` | `crates/pellucid-tauri/src/main.rs` — Tauri 2 commands, tray, sidecar spawn, vault, updater. **Inherited bug H1 fixed (token rotation, §24).** |
| `src-tauri/sidecar/local-api-server.mjs` (1580 LOC, Node) | `crates/pellucid-sidecar-bin` (Rust, Axum). Same dynamic-port HTTP server, same IPC token gate, same 14-stage gateway, same cloud-fallback, but native Rust, no Node, no monkey-patched fetch |
| `api/` (Vercel Edge JS) | Removed. Public API now served by `crates/pellucid-edge-bin` (Axum) hitting same `pellucid-handlers` crate that the sidecar uses |
| `server/gateway.ts` (509 LOC) | `crates/pellucid-gateway/src/lib.rs` — same 14-stage pipeline as a `Tower` middleware stack |
| `server/router.ts` | `crates/pellucid-gateway/src/router.rs` — static `phf` map + dynamic param scan + POST→GET compat |
| `server/_shared/redis.ts` (366 LOC) | `crates/pellucid-cache/src/lib.rs` — SQLite-backed KV with stampede coalescing, negative sentinel, batch pipeline |
| `server/_shared/auth-session.ts` | `crates/pellucid-auth/src/clerk.rs` — `jwtVerify` against Clerk JWKS using `jsonwebtoken` crate |
| `server/_shared/entitlement-check.ts` | `crates/pellucid-auth/src/entitlement.rs` — Redis(SQLite) cache → Convex HTTP fallback. **Inherited bug H2 fixed (distinct 503 + Retry-After on upstream-down vs 403 on genuinely unauthorized, §24).** |
| `server/_shared/rate-limit.ts` (Upstash sliding window) | `crates/pellucid-cache/src/rate_limit.rs` — sliding window over SQLite with Lua-equivalent transactional update. **Inherited M8 fixed (umbrella aggregate cap).** |
| `server/worldmonitor/<domain>/v1/*` | `crates/pellucid-handlers/src/<domain>/v1/*.rs` — same RPC names, same cache keys (with linter for M1 fix), same envelope |
| `scripts/ais-relay.cjs` (10891 LOC) | `crates/pellucid-streams/` (lib) + `crates/pellucid-relay-bin/` (deployed binary). All upstream clients ported (AIS via `tokio-tungstenite`, OpenSky via `oauth2` + `reqwest`, RSS via `reqwest` + `feed-rs`, Telegram via `grammers-client`, OREF via `reqwest` with custom JA3 fingerprint via `rustls`/`ja3-rustls`). **Inherited Critical C1 fixed (hard-fail startup if `RELAY_SHARED_SECRET` unset in prod, §24).** |
| `scripts/_seed-utils.mjs` (994 LOC) | `crates/pellucid-seeders/src/atomic_publish.rs` — same lock → validate → envelope → 5 MB check → staging → canonical → meta → release flow; SQLite uses transactional `BEGIN IMMEDIATE` instead of Redis `SET NX` |
| `scripts/seed-*.mjs` (~140 seeders) | `crates/pellucid-seeders/src/<domain>/*.rs` — one Rust module per source seeder, scheduled by `pellucid-seeders/src/scheduler.rs` |
| `scripts/notification-relay.cjs` | `crates/pellucid-relay-bin` (extra task in same binary) |
| `scripts/scenario-worker.mjs`, `process-deep-forecast-tasks.mjs`, `process-simulation-tasks.mjs` | `crates/pellucid-workers/src/{scenario,deep_forecast,simulation}.rs` |
| `scripts/build-sidecar-sebuf.mjs`, `build-sidecar-handlers.mjs` | `crates/pellucid-codegen/` (Rust build script invoked by `cargo xtask gen`) |
| `scripts/desktop-package.mjs`, `sync-desktop-version.mjs` | `bunx scripts/release.ts` (Bun-driven) plus `cargo-tauri-action` |
| `scripts/lint-boundaries.mjs`, `check-unicode-safety.mjs` | Bun scripts in `tools/` |
| `proto/` (sebuf annotations) | **Retained as-is**; `make generate` becomes `bun run gen` which runs `buf generate` + Rust codegen plugin emitting `crates/pellucid-handlers/src/generated/` |
| `convex/` | **Retained**; only `schema.ts` (waitlist + contact) plus billing/webhook actions remain |
| `middleware.ts` (Vercel Edge) | `crates/pellucid-edge-bin/src/middleware.rs` — bot UA filter, social-preview UA allowlist, CSP headers (and **inherited M5 fixed** by single-source CSP builder, §24) |
| Husky pre-push hook | `.husky/pre-push` rewritten as Bun script: `bun run check` (typecheck + cargo check + clippy + lint + edge-import guardrail + markdown + version sync). **OP-19** preserved |

---

## 4. Topology

```text
                 ┌──────────────────────────── PELLUCID ────────────────────────────┐
                 │                                                                   │
       ┌─────────┴───────────────┐                              ┌─────────────────┐ │
       │ Tauri Desktop App        │                              │ Browser users   │ │
       │  webview (React + Vite)  │  IPC + 127.0.0.1:dyn         │  (worldmonitor  │ │
       │     ┌─────────────────┐  │                              │   .app SPA)     │ │
       │     │ pellucid-tauri  │──┼─► spawns pellucid-sidecar-bin│                 │ │
       │     │   (Rust main)   │  │   (Rust, Axum, in-process)   │                 │ │
       │     └─────────────────┘  │                              └────────┬────────┘ │
       │                          │                                       │ HTTPS    │
       │     SQLite (per-user     │                                       ▼          │
       │     ~/Library/Application│                              ┌─────────────────┐ │
       │     Support/pellucid.db) │                              │ pellucid-edge   │ │
       │                          │                              │  -bin (Rust)    │ │
       └─────────┬────────────────┘                              │  Railway        │ │
                 │                                               │  (or self-host) │ │
                 │ optional cloud sync (entitlements,            │  Axum + Tower   │ │
                 │  shared layouts) over HTTPS                   │  api.worldmoni- │ │
                 ▼                                               │  tor.app        │ │
       ┌──────────────────────┐                                  └────────┬────────┘ │
       │  Convex Cloud        │ ◄─── Dodo webhooks (HMAC verify)          │          │
       │  (entitlements +     │      Clerk JWKS                           │          │
       │   webhook receiver + │                                           │          │
       │   sync)              │                                  ┌────────▼────────┐ │
       └──────────────────────┘                                  │ SQLite          │ │
                                                                 │ (server-side    │ │
                                                                 │  per region;    │ │
                                                                 │  Litestream-    │ │
                                                                 │  backed)        │ │
                                                                 └────────┬────────┘ │
                                                                          │          │
       ┌──────────────────────┐                                           │          │
       │ pellucid-relay-bin   │ ◄─── x-relay-key Bearer (req'd in prod)   │          │
       │ (Rust)               │                                           │          │
       │ Railway              │                                           │          │
       │  - AIS WS            │      writes envelope                      │          │
       │  - OpenSky OAuth2    │ ────────────────────────────────────────► ┘          │
       │  - RSS allowlist     │                                                      │
       │  - Telegram MTProto  │                                                      │
       │  - OREF JA3-bypass   │                                                      │
       │  - ~20 seed loops    │                                                      │
       └──────────────────────┘                                                      │
                                                                                     │
       30+ upstream APIs (aviationstack, BLS, ECB, EIA, FRED, IMF, OpenMeteo, OpenSky,│
       PortWatch, UCDP, ACLED, NASA EONET, USGS, GDELT, Yahoo, Polygon, NOAA, etc.)  │
       ────────────────────────────────────────────────────────────────────────────────┘
```

Three deployable Rust binaries:

1. **`pellucid-sidecar-bin`** — spawned by Tauri main, listens on `127.0.0.1:<dyn-port>`, serves the local UI.
2. **`pellucid-edge-bin`** — deployed to **Railway** (or self-host), fronts `api.worldmonitor.app` and serves the hosted web SPA's RPC calls.
3. **`pellucid-relay-bin`** — deployed to **Railway**, runs the stream clients and seed loops, writes envelopes to the same canonical SQLite (server-side replica).

A fourth in-process Rust target — `pellucid-tauri` — is the Tauri 2 host (system tray, updater, vault, IPC commands).

All three binaries link the same crate workspace (`pellucid-core`, `pellucid-cache`, `pellucid-gateway`, `pellucid-handlers`, `pellucid-auth`, `pellucid-ml`, `pellucid-correlation`), so behavior is bit-identical regardless of where the request is served.

---

## 5. Module Boundary: Public vs Non-Public

The user's mandate — *"Rust for the systems that aren't public facing"* — is satisfied as follows.

### 5.1 Public-facing (TypeScript)

- `webview/` — the Vite + React + Tailwind + Radix + Zustand bundle.
  - Same bundle ships in Tauri webview AND is served at `worldmonitor.app`.
  - Single source of truth for UI; build-time `VITE_TARGET=desktop|web` flips runtime behavior.
- The hosted web SPA is served as static assets by `pellucid-edge-bin` (Axum `tower-http::services::ServeDir`).

### 5.2 Non-public-facing (Rust)

Everything that processes data, hits upstreams, manages cache, runs ML, evaluates entitlements, or schedules seeds — runs in Rust. Detail in §11 (workspace).

### 5.3 Retained external SaaS

- **Clerk** — authentication. Pellucid uses Clerk's React SDK in the webview and verifies JWTs with Rust `jsonwebtoken` crate against Clerk JWKS in `pellucid-auth`.
- **Dodo** — payments. Webhook events arrive at Convex, signature-verified by Dodo's HMAC scheme, with HMAC-signed `wm_user_id_sig` (OP-12) preserved.
- **Convex** — minimal retention: schema (contact + waitlist + entitlements snapshot), webhook actions, identity-signing helpers, `internal-entitlements` HTTP endpoint that `pellucid-edge-bin` falls back to on cache miss.
- **Sentry** — error reporting (browser + Rust via `sentry-rust`).

---

## 6. Data Model — SQLite as Canonical Store

SQLite replaces Upstash Redis in every role. Configuration mandates:

```sql
PRAGMA journal_mode = WAL;
PRAGMA synchronous = NORMAL;
PRAGMA busy_timeout = 5000;
PRAGMA temp_store = MEMORY;
PRAGMA mmap_size = 268435456;          -- 256 MiB
PRAGMA foreign_keys = ON;
PRAGMA cache_size = -65536;            -- 64 MiB
```

### 6.1 Core tables

```sql
-- Cache envelope: replaces Redis SET/GET of envelope payloads.
CREATE TABLE IF NOT EXISTS kv_envelope (
    cache_key       TEXT PRIMARY KEY,
    payload         TEXT NOT NULL,            -- JSON; contains {_seed, data} when enveloped
    fetched_at_ms   INTEGER NOT NULL,
    ttl_ms          INTEGER NOT NULL,
    record_count    INTEGER,
    source_version  TEXT,
    state           TEXT,                     -- 'live' | 'stale' | 'backup' | 'seeded'
    is_negative     INTEGER NOT NULL DEFAULT 0  -- negative-cache sentinel (was NEG_SENTINEL)
) WITHOUT ROWID;

CREATE INDEX kv_envelope_fetched ON kv_envelope (fetched_at_ms);

-- Seed metadata: replaces Redis seed-meta:* keys.
CREATE TABLE IF NOT EXISTS seed_meta (
    cache_key       TEXT PRIMARY KEY,
    fetched_at_ms   INTEGER NOT NULL,
    ttl_ms          INTEGER NOT NULL,
    last_run_id     TEXT NOT NULL,
    source_version  TEXT,
    record_count    INTEGER,
    cascade_group   TEXT                       -- e.g. 'theater-posture' for cascade tolerance
) WITHOUT ROWID;

-- Distributed locks: replaces SET NX PX seed-lock:*.
CREATE TABLE IF NOT EXISTS seed_lock (
    domain          TEXT PRIMARY KEY,
    run_id          TEXT NOT NULL,
    expires_at_ms   INTEGER NOT NULL
) WITHOUT ROWID;

-- Entitlement cache: replaces entitlements:${ENV_PREFIX}:${userId} 15-min TTL.
CREATE TABLE IF NOT EXISTS entitlements_cache (
    user_id         TEXT PRIMARY KEY,
    tier            INTEGER NOT NULL,
    features_json   TEXT NOT NULL,
    valid_until_ms  INTEGER NOT NULL,
    cached_at_ms    INTEGER NOT NULL
) WITHOUT ROWID;

-- Rate limit windows: replaces Upstash sliding window.
CREATE TABLE IF NOT EXISTS rate_limit_window (
    bucket_key      TEXT NOT NULL,             -- e.g. 'rl:ip:1.2.3.4' or 'rl:ep:summarize:...'
    request_at_ms   INTEGER NOT NULL,
    PRIMARY KEY (bucket_key, request_at_ms)
);

CREATE INDEX rate_limit_lookup ON rate_limit_window (bucket_key, request_at_ms);

-- User-side preferences (desktop and synced).
CREATE TABLE IF NOT EXISTS panel_layout (
    user_id         TEXT NOT NULL,
    panel_id        TEXT NOT NULL,
    variant         TEXT NOT NULL,
    layout_json     TEXT NOT NULL,
    updated_at_ms   INTEGER NOT NULL,
    PRIMARY KEY (user_id, panel_id, variant)
) WITHOUT ROWID;

-- Webhook idempotency.
CREATE TABLE IF NOT EXISTS webhook_seen (
    webhook_id      TEXT PRIMARY KEY,
    received_at_ms  INTEGER NOT NULL,
    payload_hash    TEXT NOT NULL
) WITHOUT ROWID;
```

### 6.2 Specialized virtual tables

```sql
-- News + intel full-text search.
CREATE VIRTUAL TABLE news_fts USING fts5(
    article_id UNINDEXED, title, summary, body, source, lang,
    tokenize = 'unicode61 remove_diacritics 2'
);

-- Vessel + aircraft positions for spatial queries (replaces geoSearchByBox).
CREATE VIRTUAL TABLE positions_rtree USING rtree(
    id,
    min_lon, max_lon,
    min_lat, max_lat
);
CREATE TABLE positions_meta (
    id INTEGER PRIMARY KEY,
    kind TEXT NOT NULL,                        -- 'vessel' | 'aircraft' | 'satellite'
    external_id TEXT NOT NULL,
    payload_json TEXT NOT NULL,
    observed_at_ms INTEGER NOT NULL
);

-- Embeddings for semantic search (sqlite-vec).
CREATE VIRTUAL TABLE embeddings USING vec0(
    embedding FLOAT[384]                       -- MiniLM-L6 dim
);
CREATE TABLE embedding_meta (
    rowid INTEGER PRIMARY KEY REFERENCES embeddings(rowid),
    article_id TEXT NOT NULL,
    model TEXT NOT NULL,
    created_at_ms INTEGER NOT NULL
);
```

### 6.3 Cache-key conventions (preserved from `LoreDeepCodeReview.md §1.4`)

- Param-scoped: `aviation:status:${flight}:${date}:${origin}:v1`
- Hardcoded for seeded: `market:stocks-bootstrap:v1`
- Daily: `climate:anomalies:list:v1`
- A linter (`tools/check-cache-keys.ts`, Bun) parses every `cachedFetchJson` call in `crates/pellucid-handlers/src/**/*.rs` and fails CI if a request param is referenced in the handler body but absent from the key string. **(Inherited M1 fix.)**

### 6.4 Server-side replica + durability

- Single SQLite file per region, fronted by `pellucid-edge-bin`.
- Continuous replication via **Litestream** to S3-compatible storage (Cloudflare R2 default).
- Read replicas optional via **LiteFS** if multi-region read scaling becomes a need; not required for v1.

### 6.5 Desktop-side store

- `~/Library/Application Support/Pellucid/pellucid.db` (macOS), platform equivalents elsewhere.
- Same schema. Tauri vault (consolidated keychain entry, `pellucid-tauri/src/vault.rs`) holds API keys + Clerk session token.
- Optional opt-in cloud sync of `panel_layout` and `entitlements_cache` via Convex.

---

## 7. Cache Hierarchy & Freshness

### 7.1 Six tiers — preserved exactly

```rust
// crates/pellucid-gateway/src/cache_tiers.rs
pub struct TierHeaders {
    pub s_maxage:      u32,
    pub stale_while_revalidate: u32,
    pub stale_if_error: u32,
}

pub const FAST:         TierHeaders = TierHeaders { s_maxage:   300, stale_while_revalidate:   60, stale_if_error:   1200 };
pub const MEDIUM:       TierHeaders = TierHeaders { s_maxage:   600, stale_while_revalidate:  120, stale_if_error:   1800 };
pub const SLOW:         TierHeaders = TierHeaders { s_maxage:  1800, stale_while_revalidate:  300, stale_if_error:   7200 };
pub const SLOW_BROWSER: TierHeaders = TierHeaders { s_maxage:   900, stale_while_revalidate:   60, stale_if_error:   1800 };
pub const STATIC:       TierHeaders = TierHeaders { s_maxage:  3600, stale_while_revalidate:  600, stale_if_error:  28800 };
pub const DAILY:        TierHeaders = TierHeaders { s_maxage: 86400, stale_while_revalidate: 3600, stale_if_error: 172800 };
pub const NO_STORE:     TierHeaders = TierHeaders { s_maxage:     0, stale_while_revalidate:    0, stale_if_error:      0 };
```

Premium paths force `SLOW_BROWSER` override; per-RPC env override `CACHE_TIER_OVERRIDE_${RPC_NAME}` is preserved (OP-21).

### 7.2 Stampede protection

`crates/pellucid-cache/src/coalesce.rs` keeps an `Arc<Mutex<HashMap<String, Shared<JoinHandle>>>>` of in-flight fetches. Concurrent miss on the same key awaits the existing future, ensuring a single upstream request and a single SQLite write. Direct port of the Redis `inFlight` Map (`server/_shared/redis.ts:198,215-216`).

### 7.3 Negative-cache sentinel

A row with `is_negative = 1` and a short TTL (default 120 s) prevents null-result storms. Queries that resolve to a negative row return `Ok(None)` to the handler so it does not re-issue the upstream call.

### 7.4 Atomic seed publish (Rust port of `_seed-utils.mjs:170-210`)

```rust
pub async fn atomic_publish(
    db: &SqlitePool,
    domain: &str,
    cache_key: &str,
    envelope: SeedEnvelope,
    ttl: Duration,
) -> Result<()> {
    // 1. Acquire lock (BEGIN IMMEDIATE so sqlite serializes writers).
    let run_id = Uuid::new_v4().to_string();
    acquire_seed_lock(db, domain, &run_id, Duration::from_secs(60)).await?;

    // 2. Validate envelope shape and 5 MB cap.
    envelope.validate()?;
    let payload = serde_json::to_string(&envelope)?;
    ensure!(payload.len() <= 5 * 1024 * 1024, SizeExceeded);

    // 3. Staging row (TTL 5 min).
    sqlx::query("INSERT OR REPLACE INTO kv_envelope (cache_key, payload, fetched_at_ms, ttl_ms, ...) VALUES (?, ?, ?, ?, ...)")
        .bind(format!("{cache_key}:staging:{run_id}"))
        .bind(&payload)
        .bind(now_ms())
        .bind(300_000_i64)
        .execute(db).await?;

    // 4. Promote to canonical + write seed_meta in one transaction.
    let mut tx = db.begin().await?;
    sqlx::query("INSERT OR REPLACE INTO kv_envelope ... VALUES (?,?,?,?,...)")
        .bind(cache_key).bind(&payload).bind(now_ms()).bind(ttl.as_millis() as i64)
        .execute(&mut *tx).await?;
    sqlx::query("DELETE FROM kv_envelope WHERE cache_key = ?")
        .bind(format!("{cache_key}:staging:{run_id}"))
        .execute(&mut *tx).await?;
    sqlx::query("INSERT OR REPLACE INTO seed_meta ... VALUES (?,?,?,?,?,?,?)")
        .bind(cache_key).bind(now_ms())
        .bind(std::cmp::max(ttl, Duration::from_days(7)).as_millis() as i64)
        .bind(&run_id).bind(&envelope.source_version)
        .bind(envelope.record_count).bind(&envelope.cascade_group)
        .execute(&mut *tx).await?;
    tx.commit().await?;

    // 5. Release lock (compare-and-del).
    release_seed_lock(db, domain, &run_id).await?;
    Ok(())
}
```

---

## 8. Public API Surface (`api.worldmonitor.app`)

`pellucid-edge-bin` (Axum) exposes the same RPC contract as the original Vercel Edge handlers. Generated stubs live in `crates/pellucid-handlers/src/generated/` and are produced by `bun run gen` (which calls `buf generate` with the sebuf plugin and a Rust target plugin).

### 8.1 Top-level endpoints (exact parity with `LoreWorldMonitorComponents.md §7`)

`bootstrap`, `health`, `ais-snapshot`, `cache-purge`, `contact`, `create-checkout`, `download`, `geo`, `gpsjam`, `mcp`, `mcp-proxy`, `military-flights`, `notify`, `og-story`, `opensky`, `oref-alerts`, `polymarket`, `register-interest`, `reverse-geocode`, `rss-proxy`, `sanctions-entity-search`, `satellites`, `seed-contract-probe`, `seed-health`, `story`, `telegram-feed`, `user-prefs`, `widget-agent`.

### 8.2 Domain bundles

`aviation`, `climate`, `conflict`, `consumer-prices`, `cyber`, `data`, `discord`, `displacement`, `economic`, `eia`, `enrichment`, `forecast`, `giving`, `health`, `imagery`, `infrastructure`, `intelligence`, `maritime`, `market`, `military`, `natural`, `news`, `notification-channels`, `oauth`, `positive-events`, `prediction`, `radiation`, `research`, `resilience`, `sanctions`, `scenario`, `seismology`, `skills`, `slack`, `supply-chain`, `telegram`, `thermal`, `trade`, `unrest`, `v2`, `webcam`, `wildfire`, `youtube`. (40 domains.)

Each domain has `crates/pellucid-handlers/src/<domain>/v1/<rpc>.rs` matching the original handler 1:1. Each handler:

1. Resolves cache key (param-scoped or hardcoded — linter enforces).
2. Calls `cache::cached_fetch_json(key, tier, fetcher)`.
3. Returns `Response<Bytes>` with envelope or unwrapped data per request `Accept`.

### 8.3 Gateway pipeline (14 stages, parity with OP-3)

```rust
// crates/pellucid-gateway/src/lib.rs
pub fn build_router(handlers: HandlerSet) -> axum::Router {
    axum::Router::new()
        .merge(handlers.into_router())
        .layer(Stage14_CacheControl)
        .layer(Stage13_Etag)
        .layer(Stage12_HeaderMerge)
        .layer(Stage11_HandlerErrorBoundary)
        // Stage 10 = Router (above)
        .layer(Stage9_GlobalRateLimit)
        .layer(Stage8_EndpointRateLimit)
        .layer(Stage7_Entitlement)
        .layer(Stage6_ApiKey)
        .layer(Stage5_ClerkSession)
        .layer(Stage4_TierGate)
        .layer(Stage3_OptionsPreflight)
        .layer(Stage2_CorsMerge)
        .layer(Stage1_OriginAllowlist)
}
```

Each `Stage_*` is a `tower::Layer`. Failure modes preserved: 403 on origin, 401 on Clerk/API key, 403 on entitlement, 429 on rate, 404/405 on router, 500 on handler, 304 on ETag match.

### 8.4 ETag computation

FNV-1a over response body, set as `ETag` header. `If-None-Match` short-circuits to 304. Direct port of `server/_shared/hash.ts` to `crates/pellucid-core/src/fnv.rs`.

---

## 9. Web SPA (`worldmonitor.app`)

The same Vite + React bundle that runs in the Tauri webview is built with `VITE_TARGET=web` and served by `pellucid-edge-bin` from `tower-http::services::ServeDir` rooted at `webview/dist/`.

Differences between desktop and web build (toggled by `VITE_TARGET`):

| Concern | desktop | web |
|---|---|---|
| API base URL | `http://127.0.0.1:<dyn-port>` (resolved via Tauri IPC) | `https://api.worldmonitor.app` |
| Auth bearer | Tauri-rotated IPC token (5-min TTL, **OP-7 + H1 fix**) | Clerk session JWT |
| Persistent storage | Tauri `store` plugin → file | IndexedDB |
| Vault | macOS Keychain / Win Cred Vault / Linux secret-service | n/a (no local secrets) |
| Updater | Tauri updater (signed manifests) | service-worker version check + reload prompt |
| Variant detection | localStorage > VITE_VARIANT > hostname | hostname > VITE_VARIANT > localStorage |
| Deep linking | `tauri://` protocol | URL hash + query |

`webview/src/services/runtime.ts` retains the `toApiUrl()`, `detectDesktopRuntime()`, `installRuntimeFetchPatch()`, `installWebApiRedirect()` set, with the desktop-detection branch driven by `import.meta.env.VITE_TARGET === 'desktop'` AND the runtime `__TAURI__` global check.

CSP: triplicated source eliminated. Single Bun script `tools/build-csp.ts` emits CSP header strings for (1) `index.html` meta, (2) `pellucid-edge-bin`'s `set_csp` middleware, (3) `crates/pellucid-tauri/tauri.conf.json` `security.csp`. Pre-push hook fails if any of the three diverge from the script's output. **(Inherited M5 fix.)**

---

## 10. Desktop Shell (Tauri)

### 10.1 Layout

```
crates/pellucid-tauri/
  Cargo.toml
  tauri.conf.json
  tauri.tech.conf.json
  tauri.finance.conf.json
  tauri.commodity.conf.json
  tauri.happy.conf.json
  src/
    main.rs                    # window, tray, lifecycle
    sidecar.rs                 # spawn pellucid-sidecar-bin, port discovery
    vault.rs                   # consolidated keychain entry (OP-23)
    ipc.rs                     # invoke handlers (get_local_api_port, get_local_api_token, refresh_secrets, ...)
    updater.rs                 # signed-manifest auto-update
    token_rotation.rs          # NEW — H1 fix: 5-min token TTL with rotation
```

### 10.2 IPC commands exposed to webview

| Command | Purpose | Replaces |
|---|---|---|
| `get_local_api_port` | Discover dynamic port of sidecar | `runtime.ts:30-49` |
| `get_local_api_token` | Fetch current bearer (rotated every 5 min) | `main.rs:242-252` |
| `refresh_secrets` | Reload vault if user rotated keys via Keychain Access | NEW — **M9 fix** |
| `get_variant` / `set_variant` | Persisted in `store` plugin | `App.ts:424-449` |
| `request_updater_check` | Manual update check | desktop-updater |
| `open_external` | Open URL in default browser (Tauri shell allowlist) | open-external |

### 10.3 Token rotation (H1 fix, mandatory)

- On startup, generate token (32 random bytes, base64).
- Store in `LocalApiState::token` with `expires_at`.
- A `tokio::spawn` rotation task re-generates every 5 minutes and atomically swaps.
- Sidecar accepts token only if it equals current OR previous (during 30 s overlap).
- Webview observes `token_rotated` event via `tauri::Manager::emit_to` and re-issues in-flight requests with new token (the existing 401 retry path covers this; verified in regression test).

### 10.4 Vault (consolidated, with refresh)

- Single keychain entry `pellucid:secrets-vault:v1` storing JSON map of all 28 service secrets.
- Loaded once at startup, then on `refresh_secrets` IPC.
- A platform-specific keychain-change listener (macOS: `SecKeychainAddCallback`; Windows: registry-watch on Cred Vault; Linux: D-Bus `secret.Service` `CollectionChanged`) triggers automatic reload. **(M9 fix.)**

### 10.5 Sidecar spawn

`pellucid-sidecar-bin` binds `127.0.0.1:0`, prints `PORT=<n>` on stdout, then runs Axum. `pellucid-tauri::sidecar::spawn` reads stdout until it sees the port line, stores it in `LocalApiState`, and exposes via `get_local_api_port`. Fallback port `46123` only used if dynamic bind fails (matches OP-7).

---

## 11. Rust Workspace Layout

```
crates/
  pellucid-core/              # shared types, errors, FNV, envelopes, time helpers
  pellucid-db/                # SQLite handle pool, migrations (sqlx-migrate), pragmas
  pellucid-cache/             # KV API, stampede coalescing, negative sentinel, batch pipeline, rate-limit
  pellucid-streams/           # AIS, OpenSky, RSS, Telegram, OREF clients (lib only)
  pellucid-seeders/           # ~140 seeder modules, scheduler, atomic_publish
  pellucid-gateway/           # 14-stage Tower middleware stack, router, ETag, CSP middleware
  pellucid-handlers/          # 40 domains × N RPCs each; generated server stubs
  pellucid-auth/              # Clerk JWT verify, entitlement check (cache → Convex fallback), HMAC sign/verify
  pellucid-ml/                # ort or candle wrapper: embeddings, sentiment, summarization, NER
  pellucid-correlation/       # military, escalation, economic, disaster adapters; cross-domain Jaccard clustering
  pellucid-workers/           # scenario, deep_forecast, simulation
  pellucid-codegen/           # build.rs: sebuf → Rust handler types + TS client stubs
  pellucid-tauri/             # Tauri 2 host (binary)
  pellucid-sidecar-bin/       # in-process desktop API binary
  pellucid-edge-bin/          # hosted public API binary
  pellucid-relay-bin/         # stream relay binary

tools/                        # Bun scripts (lint, codegen wrappers, release packaging)
webview/                      # Vite + React + Tailwind + Radix + Zustand
docs/                         # specs, ADRs, generated OpenAPI (mirrors api.worldmonitor.app)
proto/                        # buf + sebuf definitions (retained)
convex/                       # Convex schema + webhook actions (retained)
```

### 11.1 `Cargo.toml` workspace

```toml
[workspace]
resolver = "2"
members = ["crates/*"]

[workspace.dependencies]
tokio       = { version = "1", features = ["full"] }
axum        = { version = "0.7", features = ["macros", "ws"] }
tower       = "0.5"
tower-http  = { version = "0.5", features = ["cors","compression-full","trace","fs","timeout","limit"] }
reqwest     = { version = "0.12", default-features = false, features = ["rustls-tls","gzip","brotli","stream","cookies","json"] }
sqlx        = { version = "0.8", default-features = false, features = ["runtime-tokio-rustls","sqlite","macros","migrate","json","time","uuid"] }
serde       = { version = "1", features = ["derive"] }
serde_json  = "1"
thiserror   = "1"
anyhow      = "1"
tracing     = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter","json"] }
sentry      = { version = "0.34", features = ["tower","reqwest","tracing"] }
jsonwebtoken = "9"
uuid        = { version = "1", features = ["v4","v7","serde"] }
phf         = { version = "0.11", features = ["macros"] }
async-trait = "0.1"
ort         = { version = "2", optional = true }   # set in pellucid-ml as default
candle-core = { version = "0.7", optional = true } # alt backend behind feature flag
tokio-tungstenite = "0.23"
oauth2      = "4"
feed-rs     = "2"
grammers-client = "0.7"                            # Telegram MTProto
rustls      = "0.23"
ja3-rustls  = "0.1"                                 # custom JA3 fingerprint for OREF bypass
litestream  = { version = "0", optional = true }    # not a real crate; deployed as a sidecar binary
```

(The Litestream entry is illustrative — Litestream is run as a separate process, not linked in.)

### 11.2 Per-crate responsibility

- **`pellucid-core`** — `Envelope { _seed, data }`, `SeedMeta`, `CacheTier`, `FnvHasher`, `now_ms()`, error enum.
- **`pellucid-db`** — opens the SQLite pool, applies migrations (each migration is a numbered `.sql` file in `crates/pellucid-db/migrations/`), enforces pragmas. Re-exports `SqlitePool` typed alias.
- **`pellucid-cache`** — `cached_fetch_json<T, F>(key, tier, fetcher: F)` with stampede coalescing + negative sentinel; `get_cached_json_batch(keys)` (single transaction); rate-limit primitives (sliding window via SQLite, with umbrella aggregate cap — **M8 fix**).
- **`pellucid-streams`** — one module per upstream: `ais.rs`, `opensky.rs` (OAuth2 + 60 s buffer + mutex via `tokio::sync::OnceCell`), `rss.rs` (allowlist + 5 min positive / 1 min negative cache + in-flight dedup), `telegram.rs` (`grammers-client` MTProto, channel set gated by `TELEGRAM_CHANNEL_SET`), `oref.rs` (custom JA3 fingerprint), `notification.rs`. Each exposes a typed result and a `metrics::counter!(…)` increment. **No HTTP-loopback to own process — direct function calls eliminate H3.**
- **`pellucid-seeders`** — one Rust module per source `seed-*.mjs`. The scheduler at `crates/pellucid-seeders/src/scheduler.rs` registers cadences (market 5 min, aviation 30 min, NOTAM 2 h, cyber 2 h, positive 15 min, theater-posture 5 min, UCDP 30 min, corridor-risk 1 h, shipping-stress 1 h, satellite-TLEs 2 h, worldbank daily, etc.) using `tokio::time::interval`. `transient_redis_error → transient_db_error` taxonomy preserved; counter exposed at `/metrics` (**M3 fix**).
- **`pellucid-gateway`** — Tower middleware stack (§8.3), router (`phf` static + dynamic `{param}` scan + POST→GET <1 MB compat), error mapper, response headers helper. Single source of truth — both `pellucid-edge-bin` and `pellucid-sidecar-bin` mount the same router.
- **`pellucid-handlers`** — 40 domains; each handler's RPC functions take `(req: Request, state: AppState) → Result<Response, Error>`. RPC names come from sebuf-generated types in `pellucid-handlers/src/generated/`.
- **`pellucid-auth`** — Clerk JWKS fetcher (5 min refresh), `verify_jwt(token) → Claims`, `check_entitlement(user_id, required_tier) → Decision { Allow | Deny | UpstreamDown }` — **upstream-down case maps to 503 + Retry-After in gateway, not 403 (H2 fix)**. `sign_user_id_hmac` and `verify_user_id_hmac` for Dodo metadata.
- **`pellucid-ml`** — feature flag `backend-ort` (default) or `backend-candle`. API: `embed(text) → Vec<f32>`, `sentiment(text) → SentimentLabel`, `summarize(text, max_tokens) → String`, `extract_entities(text) → Vec<Entity>`. Models loaded from Tauri resource bundle at startup. ONNX models: `MiniLM-L6-v2.onnx`, `distilbert-sst2.onnx`, `bart-cnn-summary.onnx`, `xlm-roberta-ner.onnx`.
- **`pellucid-correlation`** — `Correlator` trait with four impls: `MilitaryCorrelator`, `EscalationCorrelator`, `EconomicCorrelator`, `DisasterCorrelator`. Cross-domain Jaccard clustering ported from `analysis.worker.ts`.
- **`pellucid-workers`** — async task workers (scenario evaluation, deep-forecast tasks, simulation). Each is a long-running Tokio task started by `pellucid-relay-bin`'s main.
- **`pellucid-codegen`** — build script invoked by `bun run gen`. Calls `buf generate` with two plugins: TS client (existing) and Rust handler-types (new — emits trait stubs that `pellucid-handlers/src/<domain>/v1/*.rs` implement).
- **`pellucid-tauri`** — Tauri 2 host (§10).
- **`pellucid-sidecar-bin`** — `fn main` opens SQLite, builds `AppState`, mounts `pellucid-gateway::build_router`, listens on `127.0.0.1:0`, prints `PORT=<n>`, runs Axum forever.
- **`pellucid-edge-bin`** — `fn main` opens SQLite, builds `AppState`, mounts `pellucid-gateway::build_router` plus `tower-http::ServeDir` for `webview/dist`, plus middleware (bot-UA filter, social-preview UA allowlist, CSP). Listens on `0.0.0.0:8080`. Reads its config from env (`FLY_REGION`, `LITESTREAM_REPLICA_URL`, `RELAY_SHARED_SECRET`, `CONVEX_SERVER_SHARED_SECRET`, `CLERK_JWKS_URL`, `DODO_*`).
- **`pellucid-relay-bin`** — `fn main` starts: AIS WS task, OpenSky token loop, RSS proxy server, Telegram poller, OREF poller, ~20 seed loops (delegated to `pellucid-seeders::scheduler`), and `/health` HTTP endpoint (Axum on `:3004`). **Hard-fails startup if `RELAY_SHARED_SECRET` is unset and `ALLOW_UNAUTHENTICATED_RELAY != "true"`. The `ALLOW_UNAUTHENTICATED_RELAY` flag refuses to coexist with `FLY_APP_NAME`/`RAILWAY_PROJECT_ID` env vars (production hostnames). C1 fix.**

### 11.3 Cross-crate guarantees

- **No I/O in `pellucid-core`** — pure types only.
- **All handler error returns route through `pellucid-gateway::error_mapper`** so logging + Sentry capture is consistent.
- **`pellucid-cache` has zero awareness of HTTP** — it's a pure key/value abstraction with stampede semantics.
- **All async tasks named** via `tokio::task::Builder::new().name(…)` for tracing.

---

## 12. UI Component Strategy

### 12.1 React 19 + Radix Primitives + Tailwind 4

- Component model is **composition over inheritance** — the original `Panel` class with `showLoading` / `showError` / `showLocked` / `showGatedCta` etc. (`Panel.ts:776-976`) becomes a single `<Panel>` component with `state` prop and named slots.
- Every panel from `LoreWorldMonitorComponents.md §3` (86 total, grouped into 9 families) is a React component under `webview/src/panels/<family>/<PanelName>.tsx`.
- Radix Primitives provide: `Dialog` (replaces `SignalModal`, `StoryModal`, `SearchModal`, `CountryIntelModal`, `MobileWarningModal`, `WidgetChatModal`, `McpConnectModal`), `Popover` (`MapPopup`, `MapContextMenu`), `Tooltip`, `DropdownMenu`, `Tabs`, `ToggleGroup`, `Toast` (replaces `BreakingNewsBanner` + `payment-failure-banner`), `ScrollArea` (`VirtualList` host), `Collapsible` (panel collapse), `Slider` (`PlaybackControl`), `Toolbar` (panel header), `Toggle` (variant switcher).
- Tailwind 4 configured with the Radix Colors plugin; per-variant theme overrides in `webview/src/styles/variants/{base,tech,finance,commodity,happy}.css`.

### 12.2 `<Panel>` shape

```tsx
interface PanelProps {
  id: PanelId;
  title: string;
  state: 'loading' | 'error' | 'locked' | 'gated' | 'retrying' | 'config-error' | 'ready';
  count?: number;
  badge?: { kind: 'live' | 'new' | 'stale'; text?: string };
  severity?: SeverityLevel;
  footer?: ReactNode;
  onClose?: () => void;
  onCollapse?: () => void;
  children: ReactNode;
}
```

State views (`showLoading`, `showError`, etc.) become render branches inside the component. `setContent(html)` and the 150 ms debounce are gone — replaced by React reconciliation + `useDeferredValue` for content updates that arrive at high frequency.

### 12.3 Map components

- `<DeckGLMap>` — wraps `deck.gl` `Deck` instance keyed to a `<canvas>` ref, props for layers (`Scatterplot`, `GeoJson`, `Path`, `Icon`, `Polygon`, `Arc`, `Heatmap`, `H3Hexagon`), PMTiles via `pmtiles-protocol`, supercluster for points-of-interest. State pushed via Zustand `useMapStore`.
- `<GlobeMap>` — wraps `globe.gl`. `htmlElementsData` merged with `_kind` discriminator preserved (OP-15).
- `<MapContainer>` switches between 2D and 3D based on `useMapStore.mode`.

### 12.4 Virtualization

- `VirtualList` becomes `react-virtuoso` with custom item renderer.
- Heavy panels (`LiveNewsPanel`, `GdeltIntelPanel`, `TelegramIntelPanel`, `UcdpEventsPanel`) use Virtuoso for list rendering.

### 12.5 Frontend boundaries

- Pure render components import only Tailwind classes, Radix primitives, and Zustand selectors.
- No fetch calls inside components — data comes from Zustand stores hydrated by `data/loader.ts`.
- Worker-style tasks (correlation, ML) are now Rust IPC calls — the webview never bundles `@xenova/transformers` or `onnxruntime-web`.

---

## 13. State Management — Zustand

### 13.1 Store split

| Store | Slice content | Persistence |
|---|---|---|
| `useAuthStore` | `{ user, clerkSessionToken, entitlements }` | session only |
| `useVariantStore` | `{ variant, available, switching }` | `localStorage` (web) / Tauri store (desktop) |
| `useUiStore` | `{ theme, lang, sidebarOpen, modalStack }` | `localStorage` / Tauri store |
| `usePanelStore` | `{ panels: Map<id, PanelRuntime>, layout, hidden }` | `panel_layout` table via Tauri command (desktop) / IndexedDB (web) |
| `useDataStore` | `{ byPanel: Map<PanelId, PanelData>, lastFetchedAt: Map<…> }` | session only |
| `useMapStore` | `{ mode: '2d' \| '3d', viewport, layers, selection }` | URL (debounced 250 ms) — port of `urlState.ts` |
| `useNewsStore` | `{ feed, breaking, gaps, signals }` | session only |
| `useCorrelationStore` | `{ active, results, lastRunAt }` | session only |
| `useBootStore` | 8-phase state machine for the `init()` flow | session only |

### 13.2 Subscriptions and selectors

- All stores use `subscribeWithSelector` middleware so panel components subscribe only to their own slice.
- Cross-store reactions (e.g., variant change → reset map layers, disable cross-variant panels) live in `webview/src/state/reactions.ts` (port of `App.ts:424-449`).

### 13.3 URL state sync

- `useMapStore` writes viewport + selection to URL via debounced 250 ms `setUrlState` (port of `src/utils/urlState.ts`).
- `webview/src/state/url-sync.ts` subscribes to `useMapStore` + `useVariantStore` + `useUiStore` and serializes a compact base64-encoded state object.

---

## 14. Auth & Entitlement

### 14.1 Flow (parity with `LoreDeepCodeReview.md §1.5`)

```
Browser/Webview
  │  Clerk session token (template="convex" with plan claim)
  ▼
pellucid-edge-bin or pellucid-sidecar-bin
  │  jwtVerify (jsonwebtoken crate, JWKS cached 5 min)
  │  payload.sub = user_id → injected as x-user-id header
  ▼
pellucid-auth::check_entitlement(user_id, required_tier)
  │  1. SQLite entitlements_cache (15 min TTL, valid_until check)
  │  2. miss → POST {CONVEX_SITE_URL}/api/internal-entitlements (CONVEX_SERVER_SHARED_SECRET)
  │  3. compare features.tier ≥ ENDPOINT_ENTITLEMENTS[pathname]
  ▼
Decision::Allow | Deny | UpstreamDown
  │
  ├─ Allow → handler
  ├─ Deny → 403
  └─ UpstreamDown → 503 + Retry-After: 30   ← H2 fix
```

### 14.2 Tier map (preserved)

```rust
// crates/pellucid-auth/src/endpoint_tiers.rs
pub static ENDPOINT_ENTITLEMENTS: phf::Map<&str, u8> = phf::phf_map! {
    "/api/market/v1/analyze-stock"             => 2,
    "/api/market/v1/get-stock-analysis-history"=> 2,
    "/api/market/v1/backtest-stock"            => 2,
    "/api/market/v1/list-stored-stock-backtests"=> 2,
};
```

### 14.3 Plan catalog (Convex retained)

- `convex/config/productCatalog.ts` retained verbatim.
- Tiers: `free(0)` → `pro_monthly`/`pro_annual`(1) → `api_starter`(2)/`api_business`(2) → `enterprise`(3).
- Features: `{ tier, maxDashboards, apiAccess, apiRateLimit, prioritySupport, exportFormats }` — surfaced to Pellucid via `internal-entitlements` HTTP action.

### 14.4 Legacy `PREMIUM_RPC_PATHS` retirement (H4 fix)

- Status quo (33 paths Bearer `role='pro'`) **not** ported. Instead, `ENDPOINT_ENTITLEMENTS` becomes a strict superset on day one — the migration script `tools/migrate-premium-paths.ts` (Bun) emits the additional 33 entries with their tier mapping.
- Single gating path enforced by gateway; legacy code path absent from `pellucid-gateway` from v1.

### 14.5 Identity signing for Dodo

- `pellucid-auth::sign_user_id_hmac(user_id, DODO_IDENTITY_SIGNING_SECRET) → base64` ported from `convex/lib/identitySigning.ts:29-50` using `hmac` + `sha2` crates with constant-time compare via `subtle::ConstantTimeEq`.

---

## 15. Payments — Convex + Dodo retained

- Webhook events arrive at `${CONVEX_SITE_URL}/dodopayments-webhook` (`webhookHandlers.ts:17-48`). Verification uses `webhook-id`, `webhook-timestamp`, `webhook-signature` headers and HMAC-SHA256 over the canonical body (`verifyWebhookPayload`).
- Idempotency: `webhook_seen` table in Convex keyed by `webhook-id` (OP-18).
- HMAC-signed `wm_user_id_sig` in checkout `metadata` prevents client-side spoofing (OP-12).
- Successful payment events update Convex entitlement records; `pellucid-edge-bin` reads them via `internal-entitlements` HTTP action with `x-convex-shared-secret`.
- Checkout creation: webview calls `POST /api/create-checkout` → `pellucid-edge-bin` proxies to `${CONVEX_SITE_URL}/relay/create-checkout` with `Authorization: Bearer ${RELAY_SHARED_SECRET}` (constant-time compare on receive).

---

## 16. Variant System (preserved)

### 16.1 Detection chain (OP-13)

```ts
// webview/src/config/variant.ts
function detectVariant(): Variant {
  const fromBuild = (import.meta.env.VITE_VARIANT as Variant) ?? null;
  const fromHost  = matchHostnamePrefix(window.location.hostname); // tech./finance./commodity./happy.
  const fromStore = isDesktop() ? store.get('variant') : localStorage.getItem('variant');
  return fromBuild ?? fromHost ?? (fromStore as Variant) ?? 'base';
}
```

### 16.2 Configs

- `webview/src/config/variants/{base,tech,finance,commodity,happy}.ts` — same shape as original.
- Variant change reaction (port of `App.ts:424-449`): reset `useMapStore.layers`, disable panels not in target variant's allow-list, seed defaults, record migration keys (`PANEL_KEY_RENAMES_MIGRATION_KEY`, `UNIFIED_MIGRATION_KEY`, `HAPPY_PANEL_FIX_KEY`).

### 16.3 Build matrix

```jsonc
// package.json (webview)
"scripts": {
  "build:full":      "VITE_VARIANT=base       VITE_TARGET=web vite build",
  "build:tech":      "VITE_VARIANT=tech       VITE_TARGET=web vite build",
  "build:finance":   "VITE_VARIANT=finance    VITE_TARGET=web vite build",
  "build:commodity": "VITE_VARIANT=commodity  VITE_TARGET=web vite build",
  "build:happy":     "VITE_VARIANT=happy      VITE_TARGET=web vite build",
  "build:desktop":   "VITE_TARGET=desktop vite build && cargo tauri build"
}
```

`pellucid-edge-bin` serves the variant matching the request hostname from a multi-rooted `ServeDir` map.

### 16.4 OG metadata

- The middleware-served social-preview HTML for crawlers (`middleware.ts:30-55`) ports to `crates/pellucid-edge-bin/src/social_preview.rs`. Same Twitter / Facebook / LinkedIn / Telegram / Discord UA allowlist.

---

## 17. Streams & Seeders (the relay rewrite)

### 17.1 Architecture

```
crates/pellucid-relay-bin/src/main.rs
  └── starts:
        - pellucid-streams::ais::run(state)
        - pellucid-streams::opensky::run(state)
        - pellucid-streams::rss::run(state)
        - pellucid-streams::telegram::run(state)
        - pellucid-streams::oref::run(state)
        - pellucid-seeders::scheduler::run(state, registry)
        - axum server on :3004 with /health, /metrics, /opensky proxy
```

### 17.2 AIS

- `tokio-tungstenite` client to `wss://stream.aisstream.io/v0/stream`.
- API key in connection auth, exponential backoff reconnect, HIGH/LOW watermark queue (preserved from `ais-relay.cjs:48-58, 7100-7200`).
- Decoded messages flow into a `tokio::sync::broadcast::channel` so multiple consumers (panel snapshot, FTS index) get the stream once.

### 17.3 OpenSky

- `oauth2` crate, client_credentials flow.
- Token cached with `tokio::sync::OnceCell` and 60 s buffer before expiry.
- Mutex serializes refresh (no thundering herd).
- LRU positive cache (`mini-moka` crate, 128 entries, 60 s TTL).
- Negative sentinel (30 s).
- 90 s cooldown on 429.

### 17.4 RSS

- Allowlist from `crates/pellucid-streams/src/rss/allowed_domains.rs` (ported from `shared/rss-allowed-domains.cjs`).
- 5 min positive / 1 min negative cache.
- In-flight dedup via `dashmap`.
- Feed parsing via `feed-rs`.

### 17.5 Telegram

- `grammers-client` MTProto.
- StringSession persisted at `data/telegram.session` (encrypted via `pellucid-tauri::vault` on desktop; encrypted-at-rest via Fly Secrets on edge).
- Channel set from `data/telegram-channels.json`, gated by `TELEGRAM_CHANNEL_SET` env.
- 60 s poll, 15 s per-channel timeout.

### 17.6 OREF

- `reqwest` with custom-built `rustls` ClientConfig using `ja3-rustls` to spoof a Chrome JA3 fingerprint (replaces curl JA3 bypass in `ais-relay.cjs`).
- Residential proxy fallback via `reqwest::Proxy` if direct fails.
- History persisted in `kv_envelope` with key `relay:oref:history:v1`.

### 17.7 Seed loops

- Scheduler at `crates/pellucid-seeders/src/scheduler.rs` registers cadences from a static `phf::Map<&str, Cadence>` and dispatches workers via `tokio::time::interval`.
- Cadences identical to source (market 5 min, aviation 30 min, NOTAM 2 h, …).
- Each seeder runs `atomic_publish` (§7.4).
- Theater-posture seeder (`pellucid-seeders::theater_posture`) calls `pellucid-streams::opensky::fetch_box(bbox)` **directly in-process** — no localhost loopback. **(H3 fix — eliminates the 30 s startup-delay race.)**

### 17.8 Hard-fail on missing secret (C1 fix)

```rust
// crates/pellucid-relay-bin/src/main.rs
fn main() -> Result<()> {
    let secret = std::env::var("RELAY_SHARED_SECRET").ok();
    let allow_unauth = std::env::var("ALLOW_UNAUTHENTICATED_RELAY").as_deref() == Ok("true");
    let prod_indicator = std::env::var("FLY_APP_NAME").is_ok()
        || std::env::var("RAILWAY_PROJECT_ID").is_ok()
        || std::env::var("PELLUCID_PROD").as_deref() == Ok("true");

    match (secret.as_deref(), allow_unauth, prod_indicator) {
        (Some(s), _, _) if !s.is_empty() => { /* normal authorized startup */ }
        (_, true, false)                 => { tracing::warn!("relay running unauthenticated (dev only)"); }
        _ => bail!("RELAY_SHARED_SECRET unset in production; refusing to start"),
    }
    // ...
}
```

### 17.9 Relay `/health` endpoint (L3 fix)

- Real handler at `crates/pellucid-relay-bin/src/health.rs` — returns 200 with seed_meta freshness summary, 503 if any cascade-tagged group has zero healthy members.
- Dockerfile healthcheck (`docker/Dockerfile.relay`) verified by an integration test that boots the binary and curls the endpoint.

---

## 18. ML Strategy

### 18.1 Backend selection

- Default: **`ort` 2.x** (ONNX Runtime Rust bindings) — lowest port risk, mature, broad model support.
- Alt feature flag: **`candle`** (HF native Rust) — for builds that don't want the ONNX C++ runtime in the binary. Selected with `cargo build --features pellucid-ml/candle --no-default-features`.

### 18.2 Models bundled

| Task | Model | Source |
|---|---|---|
| Sentence embeddings | `MiniLM-L6-v2.onnx` | preserves `@xenova/transformers` MiniLM-L6 from `LoreWorldMonitorComponents.md §5` |
| Sentiment | `distilbert-sst2.onnx` | port of transformers.js sentiment task |
| Summarization | `bart-cnn-summary.onnx` (or `t5-small.onnx`) | port of transformers.js summarization |
| NER | `xlm-roberta-ner.onnx` | port of transformers.js NER |

Models live in `crates/pellucid-ml/models/` and are copied into the Tauri resource bundle for desktop, downloaded on first use for edge (avoids inflating Docker image).

### 18.3 API

```rust
// crates/pellucid-ml/src/lib.rs
pub trait MlEngine: Send + Sync {
    fn embed(&self, text: &str) -> Result<Vec<f32>>;
    fn batch_embed(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>>;
    fn sentiment(&self, text: &str) -> Result<Sentiment>;
    fn summarize(&self, text: &str, max_tokens: usize) -> Result<String>;
    fn extract_entities(&self, text: &str) -> Result<Vec<Entity>>;
}
```

Engine instantiated once at sidecar/edge startup, accessed by handlers and the correlation engine. Webview reaches it via Tauri IPC commands `ml_embed`, `ml_sentiment`, etc., or via the public RPC endpoints under `/api/intelligence/v1/*`.

### 18.4 Vector search

`embeddings` virtual table (sqlite-vec) replaces the IndexedDB-backed `vector-db.ts` from `LoreWorldMonitorComponents.md §5`. Query example:

```sql
SELECT embedding_meta.article_id, distance
FROM embeddings
JOIN embedding_meta ON embeddings.rowid = embedding_meta.rowid
WHERE embedding MATCH :query_vec
  AND k = 20
ORDER BY distance;
```

---

## 19. Maps (preserved unchanged)

deck.gl, maplibre-gl, globe.gl, PMTiles, supercluster, h3-js — all retained as TypeScript dependencies. There is no Rust replacement and no benefit to one. The only change is the React wrapper around the imperative `Deck` and `Globe` instances.

---

## 20. Workers & Async Tasks

| Task | Original | New |
|---|---|---|
| Analysis (Jaccard clustering + cross-domain correlation) | `src/workers/analysis.worker.ts` | `pellucid-correlation` Rust crate; webview calls via IPC `correlation_run` |
| ML inference | `src/workers/ml.worker.ts` (transformers.js) | `pellucid-ml` Rust crate; IPC `ml_*` commands |
| Vector store | `src/workers/vector-db.ts` (IndexedDB) | `embeddings` SQLite vector table |
| Scenario worker | `scripts/scenario-worker.mjs` | `pellucid-workers::scenario` long-running task in `pellucid-relay-bin` |
| Deep-forecast tasks | `scripts/process-deep-forecast-tasks.mjs` | `pellucid-workers::deep_forecast` |
| Simulation | `scripts/process-simulation-tasks.mjs` | `pellucid-workers::simulation` |
| Notification relay | `scripts/notification-relay.cjs` | `pellucid-relay-bin` notification subtask |

Browser-side Web Workers are eliminated. The webview has no `*.worker.ts` files.

---

## 21. Build & Tooling

### 21.1 Bun as the JS runtime

- `package.json` `"packageManager": "bun@1.x"` — no npm, no Node.
- `bun install` replaces `npm install`.
- `bun test` replaces `node:test` for `*.test.ts` files in `webview/` and `tools/`.
- `bun run` runs scripts.
- `bunx` replaces `npx` for one-off CLI invocations.
- The blog-site `postinstall` slowdown (`L5`) is fixed by switching to `bun install --filter=blog-site` only on demand.

### 21.2 Vite 6

- `webview/vite.config.ts` — React plugin, Tailwind 4 plugin, PMTiles wasm asset handling, `VITE_TARGET` and `VITE_VARIANT` exposed.
- Dev server proxy: `/api/*` → `http://127.0.0.1:8080` (edge) or `http://127.0.0.1:46123` (sidecar) depending on `VITE_TARGET`.

### 21.3 Cargo + xtask

- Workspace built with `cargo build --workspace`.
- `cargo xtask gen` runs codegen (sebuf → handler stubs).
- `cargo clippy --workspace --all-targets -- -D warnings` runs in CI.
- `cargo nextest run --workspace` for fast parallel tests.

### 21.4 Justfile (top-level orchestrator)

```just
default:
    @just --list

install:
    bun install
    cargo fetch

gen:
    bun run gen

dev-web:
    bun run --filter=webview dev

dev-desktop:
    cargo tauri dev

dev-edge:
    cargo run -p pellucid-edge-bin

dev-relay:
    cargo run -p pellucid-relay-bin

check:
    bun run typecheck
    bun run lint
    cargo check --workspace
    cargo clippy --workspace --all-targets -- -D warnings
    bun run check-csp     # OP-19 + M5 fix
    bun run check-cache-keys  # M1 fix

test:
    bun test
    cargo nextest run --workspace
    bun run test:e2e

build-web:
    bun run --filter=webview build:full
    bun run --filter=webview build:tech
    bun run --filter=webview build:finance
    bun run --filter=webview build:commodity
    bun run --filter=webview build:happy
    cargo build --release -p pellucid-edge-bin

build-desktop:
    cargo tauri build

build-relay:
    cargo build --release -p pellucid-relay-bin
```

### 21.5 CI workflows (rebuilt parity with `LoreWorldMonitorComponents.md §14`)

- `.github/workflows/typecheck.yml` — `bun run typecheck` + `cargo check --workspace`
- `.github/workflows/lint.yml` — `bun run lint` + `cargo clippy`
- `.github/workflows/proto-check.yml` — `bun run gen` then `git diff --exit-code` (OP-20)
- `.github/workflows/build-desktop.yml` — `cargo tauri build` for macOS / Windows / Linux, signed
- `.github/workflows/docker-publish.yml` — multi-arch Docker images for `pellucid-edge-bin` and `pellucid-relay-bin` to GHCR
- `.github/workflows/test-linux-app.yml` — Playwright e2e against built desktop binary
- `.github/workflows/audit.yml` — `cargo audit` + `bun audit`

### 21.6 Pre-push hook (OP-19, parity)

```bash
# .husky/pre-push
bun run typecheck
bun run check-cjs           # not needed — keep as a no-op for compatibility
bun run check-cache-keys    # M1
bun run check-csp           # M5
bun run lint:md
bun run lint:mdx
cargo check --workspace
cargo clippy --workspace --all-targets -- -D warnings
bun run version-sync
```

---

## 22. Testing Strategy

### 22.1 Test surfaces (parity with `LoreWorldMonitorComponents.md §13`)

| Test class | Tool | Location |
|---|---|---|
| Rust unit | `cargo nextest` | `crates/<crate>/src/**/*.rs` `#[cfg(test)]` |
| Rust integration (gateway end-to-end) | `cargo nextest` + `axum::Server` test fixture | `crates/pellucid-gateway/tests/` |
| Handler contract tests | `cargo nextest` with golden envelope JSON | `crates/pellucid-handlers/tests/golden/` |
| TS unit (webview pure logic) | `bun test` | `webview/src/**/*.test.ts` |
| TS component | `bun test` + `@testing-library/react` | `webview/src/components/*.test.tsx` |
| E2E (real, per CLAUDE.md) | Playwright | `e2e/*.spec.ts` |
| E2E desktop | Playwright + Tauri driver | `e2e/desktop/*.spec.ts` |
| Visual regression (variant-scoped golden screenshots) | Playwright + `pixelmatch` | `e2e/visual/<variant>/` |
| Edge-import guardrail | Bun script | `tools/check-edge-imports.ts` (preserves `tests/edge-functions.test.mjs`) |
| Cache-invariant tests | `cargo nextest` | `crates/pellucid-cache/tests/invariants.rs` |
| Webhook idempotency tests | Convex tests + Rust receiver test | `convex/tests/` + `crates/pellucid-edge-bin/tests/webhook.rs` |
| Seed validation | `cargo nextest` per seeder | `crates/pellucid-seeders/tests/<domain>.rs` |
| Forecast eval / shadow / replay | Bun scripts | `tools/forecast-{eval,shadow,replay}.ts` |

### 22.2 E2E definition (CLAUDE.md compliance)

E2E tests in `e2e/` exercise the full stack: webview → Axum → SQLite → real upstream sandbox where available, real upstream with VCR cassette where not. **No mocks for cross-boundary calls**, per the CLAUDE.md E2E definition.

### 22.3 Regression test for every Critical/High fix

For each inherited finding fixed (C1, H1, H2, H3, H4, M1, M3, M4, M5, M8, M9, L3) — a regression test that **fails without the fix and passes with it**, per CLAUDE.md "always add a regression test that FAILS without the fix and PASSES with it." Test location: `crates/<owning-crate>/tests/regression_<finding-id>.rs`.

---

## 23. Quality Gates (must pass before claiming v1 GA)

1. `cargo clippy --workspace --all-targets -- -D warnings` clean.
2. `bun run typecheck` clean.
3. `cargo nextest run --workspace` 100% pass.
4. `bun test` 100% pass.
5. Playwright e2e (web + desktop) 100% pass on macOS, Windows, Linux.
6. Visual regression deltas ≤ 0.5% per variant.
7. Health endpoint returns HEALTHY in fresh prod-like environment for ≥ 60 s with no STALE_SEED for non-cascade keys.
8. Bootstrap roundtrip (cold cache → 112 keys hydrated) ≤ 800 ms p95 from edge, ≤ 200 ms p95 from sidecar (local SQLite).
9. Variant golden-screenshot parity vs original WorldMonitor for the 9 panel families across all 5 variants.
10. Every Critical/High finding has a passing regression test (§22.3).
11. `tools/check-cache-keys.ts` clean (M1).
12. `tools/build-csp.ts` parity check clean (M5).
13. CSP triplication test (`crates/pellucid-edge-bin/tests/csp_parity.rs` + Tauri config + `index.html`) — clean.
14. `cargo audit` and `bun audit` — no high/critical vulnerabilities.

A claim of "complete" requires producing, per CLAUDE.md: (1) exact test command and output tail, (2) `git diff --stat`, (3) typecheck/lint output, (4) explicit list of any deferred items. No claim without all four.

---

## 24. Inherited Defects to Fix on Rebuild

The source review (`LoreDeepCodeReview.md §2`) is a free audit. Every Critical and High finding is fixed **as part of v1**, not punted to v1.x. Mediums fixed where the cost is low.

### 24.1 Critical

| ID | Finding | Fix in spec |
|---|---|---|
| **C1** | Relay open-proxy when `RELAY_SHARED_SECRET` unset | §17.8 — hard-fail startup; `ALLOW_UNAUTHENTICATED_RELAY=true` refuses to coexist with prod hostname env vars; regression test in `crates/pellucid-relay-bin/tests/regression_c1.rs` |

### 24.2 High

| ID | Finding | Fix in spec |
|---|---|---|
| **H1** | Sidecar IPC token never rotates despite documented 5-min TTL | §10.3 — real rotation task; webview retry path verified; regression test in `crates/pellucid-tauri/tests/regression_h1.rs` |
| **H2** | Entitlement fail-closed indistinguishable from upstream-down | §11.2 + §14.1 — three-arm `Decision`; gateway emits 503 + `Retry-After: 30` for `UpstreamDown`; webview shows outage banner instead of upgrade prompt; regression test in `crates/pellucid-auth/tests/regression_h2.rs` |
| **H3** | Theater-posture seeder loops back through own HTTP server | §17.7 — direct in-process call to `pellucid-streams::opensky::fetch_box`; regression test in `crates/pellucid-seeders/tests/regression_h3.rs` |
| **H4** | Dual gating path (`PREMIUM_RPC_PATHS` legacy vs `ENDPOINT_ENTITLEMENTS` new) | §14.4 — single tier-based gating; legacy paths absent; migration script seeds the 33 paths into `ENDPOINT_ENTITLEMENTS` with their tier; regression test in `crates/pellucid-gateway/tests/regression_h4.rs` |

### 24.3 Medium (fixed where cost low)

| ID | Finding | Fix in spec |
|---|---|---|
| **M1** | Cache-key inconsistency | §6.3 — `tools/check-cache-keys.ts` lints handler bodies vs key strings; CI gate |
| **M2** | Sidecar IPv4-only fetch is global | §11.2 — Reqwest client per upstream with explicit `local_address` only on hostnames in `data/ipv4-required-hosts.json`; default dual-stack |
| **M3** | Seeder silent-skip on transient errors | §11.2 — counter via `metrics::counter!` exposed at `/metrics`; alerting hook |
| **M4** | Bootstrap fail-open returns partial data with 200 | `crates/pellucid-handlers/src/bootstrap/v1/get.rs` — returns 503 + `Retry-After` when `missing.len() == requested.len()`; partial returns 200 with `missing[]` (current behavior) |
| **M5** | CSP triplicated, no sync check | §9 + §21.4 — single `tools/build-csp.ts`; pre-push parity test |
| **M6** | Envelope migration incomplete | All Rust seeders default to enveloped; `is_negative` flag on bare path triggers warning; v1.1 cutoff date for bare-path removal documented in roadmap |
| **M7** | OpenSky positive cache 128 entries may thrash | §17.3 — `mini-moka` LRU sized at 1024; metrics counter; revisited at v1.1 if hit-rate < 0.6 |
| **M8** | Rate-limit prefixes disjoint, no aggregate cap | §11.2 — umbrella cap (`rl:agg:<ip>`) checked **after** specific buckets; configurable via env; documented |
| **M9** | Vault load-once, no keychain change detection | §10.4 — platform listener wakes `refresh_secrets`; manual IPC fallback |

### 24.4 Low (fixed where cheap)

| ID | Finding | Fix |
|---|---|---|
| **L1** | First-launch keyring prompts | Single consolidated entry from day one (no migration window). |
| **L2** | Stale migration keys | Fresh codebase → no migration keys to start; new ones documented in roadmap with TTL. |
| **L3** | Relay `/health` aspirational | §17.9 — real handler + integration test. |
| **L4** | Hardcoded variant CSP entries | `tools/build-csp.ts` accepts variant list from `webview/src/config/variants/`; no manual list. |
| **L5** | Blog-site postinstall slow | Removed; blog content out of scope for v1. |

---

## 25. Security Model

### 25.1 Secrets and identities

Distinct, never reused:

- `RELAY_SHARED_SECRET` — relay ↔ edge.
- `CONVEX_SERVER_SHARED_SECRET` — edge ↔ Convex `internal-entitlements`.
- `DODO_PAYMENTS_WEBHOOK_SECRET` — Dodo webhook signature key.
- `DODO_IDENTITY_SIGNING_SECRET` — HMAC-signs userId in checkout metadata.
- `CLERK_SECRET_KEY` — server Clerk verification (JWKS URL public).

All comparisons constant-time (`subtle::ConstantTimeEq` in Rust; `crypto.timingSafeEqual` in any TS code that touches a secret — there shouldn't be any after the rebuild, since the gateway is Rust).

### 25.2 CSP single source

`tools/build-csp.ts` emits a CSP header consumed by:

- `<meta http-equiv="Content-Security-Policy" …>` injected into `webview/index.html` at build time.
- `pellucid-edge-bin` `set_csp` Tower middleware.
- `crates/pellucid-tauri/tauri.conf.json` `security.csp`.

Pre-push hook fails if any of the three diverge from the script output. **(M5 fix.)**

### 25.3 Tauri CSP allowlist

- `default-src 'self'`
- `connect-src 'self' https://api.worldmonitor.app https://*.clerk.accounts.dev https://*.convex.cloud wss://stream.aisstream.io tauri://localhost http://127.0.0.1:*`
- `script-src 'self' 'wasm-unsafe-eval' https://*.clerk.accounts.dev`
- `style-src 'self' 'unsafe-inline'` (Tailwind-generated; constrained)
- `img-src 'self' data: https:`
- `frame-src` is computed from `webview/src/config/variants/*.ts` (no manual list, **L4 fix**).

### 25.4 IPC authorization

- All Tauri commands gated by `tauri::Manager::is_window_label("main")` — only the trusted window can invoke (preserved from `main.rs:242-252`).
- `get_local_api_token` returns the rotated token only to the trusted window.
- The sidecar accepts only requests with `Authorization: Bearer <token>` matching the current or previous (≤ 30 s) token.

### 25.5 Rate limiting

- Per-IP global cap (600 / 60 s sliding window) — preserved.
- Per-endpoint caps (e.g., `classify-event` 600/60 s, `summarize-article-cache` 3000/60 s) — preserved.
- Aggregate umbrella cap (default 4000/60 s per IP) checked after specifics. **(M8 fix.)**

### 25.6 Webhook verification

- Dodo: `webhook-id`, `webhook-timestamp`, `webhook-signature` HMAC-SHA256 verified via `verifyWebhookPayload` in Convex (kept).
- Idempotency: `webhook-id` lookup in `webhook_seen`.

---

## 26. Observability

- **Tracing**: `tracing` + `tracing-subscriber` JSON output in production; `tracing-tree` in dev.
- **Metrics**: `metrics` + `metrics-exporter-prometheus` exposed at `/metrics` on both edge and relay binaries.
- **Errors**: Sentry via `sentry-tower` for Axum, `sentry-rust` for the relay, `@sentry/react` in the webview.
- **Counters required**: cache hit/miss, stampede coalesce count, negative-sentinel hit count, seeder skip count, OpenSky token refresh count, ML inference latency p50/p95/p99, gateway stage failures by stage number, rate-limit blocks by bucket, webhook idempotency hits.

---

## 27. Variant Builds & Distribution

### 27.1 Desktop

- macOS arm64, macOS x86_64, Windows x86_64, Linux x86_64 (.deb, .rpm, AppImage).
- Code-signed (Apple notarization, Microsoft EV cert, Linux GPG).
- Auto-update via Tauri updater with signed manifests, served from GitHub Releases (or Cloudflare R2).
- Per-variant builds: same single binary — variant chosen at runtime from `localStorage` / Tauri store. (Eliminates the per-variant `.app` bundles in the original, simplifies distribution.)

### 27.2 Hosted

- `worldmonitor.app` apex → `pellucid-edge-bin` on Railway (3 regions: us-east, eu-west, ap-southeast).
- `tech.worldmonitor.app`, `finance.worldmonitor.app`, `commodity.worldmonitor.app`, `happy.worldmonitor.app` → same binary, hostname-resolved variant.
- `api.worldmonitor.app` → same binary, separate route map.
- Litestream replicating SQLite to Cloudflare R2 every 60 s.
- Relay binary on Railway, single region (closest to upstreams), with secondary failover.

### 27.3 Container images

- `ghcr.io/<org>/pellucid-edge:<sha>` — multi-arch (amd64, arm64).
- `ghcr.io/<org>/pellucid-relay:<sha>` — multi-arch.
- Built via `cargo zigbuild` for cross-compilation in CI.

---

## 28. Phased Roadmap

### Milestone 0 — Foundation (week 1–2)

- Workspace skeleton (`crates/`, `webview/`, `tools/`, `proto/`).
- `pellucid-core` types.
- `pellucid-db` migrations.
- `pellucid-cache` with stampede + negative sentinel + tests.
- Bun + Vite + Tailwind + Radix scaffold; `<App>` with Zustand `useAuthStore` + `useUiStore`.
- Tauri 2 host with sidecar spawn, port discovery, vault, **rotated token (H1 fix)**.
- CI workflows wired (typecheck, clippy, nextest, bun test).

**Exit criteria**: `cargo nextest run --workspace` green; `bun test` green; Tauri dev launches and webview makes one round-trip through sidecar (echo handler).

### Milestone 1 — Gateway + first vertical (week 3–5)

- `pellucid-gateway` 14-stage stack (§8.3).
- `pellucid-auth` Clerk + entitlement + 3-arm decision (**H2 fix**).
- `pellucid-handlers/aviation/v1/get-flight-status` (single canonical handler) — full path through cache, gateway, headers, ETag.
- Webview: bootstrap two-tier hydration shell (still empty data) + variant detection + 8-phase boot scaffold.
- `tools/check-cache-keys.ts` CI gate (**M1 fix**).
- Regression tests: C1 stub (relay startup gate), H1 (token rotation), H2 (503 vs 403).

**Exit criteria**: aviation panel renders flight status from desktop and from `worldmonitor.app`; entitlement 503 path verified.

### Milestone 2 — Streams + seeders (week 6–9)

- `pellucid-streams` AIS, OpenSky, RSS, OREF (Telegram deferred to M3).
- `pellucid-seeders` scheduler + 30 of the highest-priority seeders (markets, aviation, climate, conflict, energy core).
- `pellucid-relay-bin` with hard-fail startup (**C1 fix**), `/health` (**L3 fix**), `/metrics`.
- `atomic_publish` (§7.4) + tests.
- Regression test: H3 (no HTTP loopback).

**Exit criteria**: relay deployed to Railway; 30 cache keys populated; bootstrap returns hydrated data.

### Milestone 3 — Panels (the long stretch) (week 10–18)

- All 86 panels ported, family by family in this order (largest user impact first):
  1. News / intel (8 panels) — week 10
  2. Markets / finance (12 panels) — week 11
  3. Macro / economy (11 panels) — week 12
  4. Energy / commodities (6 panels) — week 13
  5. Geopolitics / military (10 panels) — week 14
  6. Climate / nature (7 panels) — week 15
  7. Infra / cyber (5 panels) — week 16
  8. Forecast / prediction (5 panels) — week 17
  9. Chat / MCP / modals / utilities / auth-billing (22 components) — week 18
- Each panel ships with: component, data-loader path, regression visual screenshot, e2e spec.
- Telegram stream brought online during week 14 (timing-aligned with intel panels).

**Exit criteria**: all 86 panels rendering with real data on all 5 variants; visual regression deltas ≤ 0.5%.

### Milestone 4 — ML + correlation (week 19–21)

- `pellucid-ml` with `ort` backend and 4 bundled models.
- `pellucid-correlation` adapters: military, escalation, economic, disaster.
- Embeddings + sqlite-vec wired to `news_fts` / `embeddings` tables.
- Webview correlation panel + signal modal use Rust IPC.

**Exit criteria**: `intelligence/v1/extract-entities`, `news/v1/search-semantic`, `correlation/v1/run` all return parity-quality results vs source product.

### Milestone 5 — Hardening + remaining fixes (week 22–24)

- All remaining Medium/Low fixes (§24.3, §24.4).
- Aggregate rate-limit cap (M8).
- IPv4-only fetch hostname allowlist (M2).
- Bootstrap all-miss → 503 (M4).
- CSP single-source (M5) and pre-push parity check.
- Bare-envelope path removed (M6).
- Vault keychain-change listener (M9).
- Visual regression sweep, performance pass.

**Exit criteria**: all Quality Gates (§23) pass.

### Milestone 6 — GA (week 25)

- Production deploy of `pellucid-edge-bin` to Railway 3-region.
- Release-signed desktop builds.
- DNS cutover from WorldMonitor.
- Public RPC docs published at `docs.worldmonitor.app` from generated OpenAPI.
- Announcement.

---

## 29. Risks & Mitigations

| Risk | Likelihood | Impact | Mitigation |
|---|---|---|---|
| ONNX Runtime cross-platform binary size + signing | Medium | High | Feature-flag `candle` backend; ship ONNX as optional download for desktop |
| 86-panel port slips | High | Medium | Family-by-family delivery (§28 M3); each family is independently shippable |
| SQLite write contention under load (server-side) | Medium | High | WAL + busy_timeout; LiteFS read replicas at v1.x; benchmark in M5 |
| Litestream replica lag during deploy | Medium | Medium | Read-only mode during cutover; documented runbook |
| Rust rewrite of `ais-relay.cjs` (10891 LOC) misses behavior | Medium | High | Module-by-module port with cassette tests against original output; behavioral diff in CI |
| Telegram MTProto Rust client (`grammers-client`) maturity | Medium | Medium | Wrap in trait + alternate `tdlib` impl behind feature flag |
| OREF JA3 fingerprint maintenance | Low | Low | Configurable via env; `ja3-rustls` actively maintained |
| Convex API drift | Low | Medium | Pin `convex` SDK; contract test on `internal-entitlements` |
| Clerk SDK changes | Low | Medium | Pin version; JWKS-only verification keeps server side stable |
| Visual regression noise (Tailwind ≠ original CSS) | High | Medium | Per-variant golden screenshots; manual review pass; ≤ 0.5% delta tolerance |
| 86 panels in one release breaks staffing assumptions | High | High | M3 schedule explicit; if family slips, ship in v1.x with feature flag rather than block GA |

---

## 30. Open Decisions Still Live

These are decisions the spec does not lock — list them here so they're tracked, not lost.

| ID | Decision | Owner | Default chosen | Override deadline |
|---|---|---|---|---|
| OD-1 | ML backend default — `ort` vs `candle` | User | `ort` (per spec §18.1) | M4 start |
| OD-2 | Hosted edge cloud — Fly.io vs Railway vs self-host | User | **Railway 3-region (locked 2026-05-04)** | resolved |
| OD-3 | Litestream replica destination — Cloudflare R2 vs S3 vs B2 | User | Cloudflare R2 | M5 start |
| OD-4 | LiteFS adoption for read replicas | User | Deferred to v1.x | v1 GA |
| OD-5 | Web SPA tenancy — single service with hostname routing vs per-variant | User | **Single Railway service, hostname routing (locked 2026-05-04)** | resolved |
| OD-6 | Telegram client — `grammers-client` vs `tdlib` Rust binding | User | `grammers-client` | M3 week 14 |
| OD-7 | Variant `happy` retention — was it experimental? Keep or sunset? | User | Keep (parity) | M3 |
| OD-8 | Pricing page rebuild — included in v1 or v1.1? | User | v1.1 | M5 |
| OD-9 | MCP-related panels (`McpConnectModal`, `McpDataPanel`, `ChatAnalystPanel`) — keep MCP integration or pivot? | User | Keep | M3 week 18 |
| OD-10 | Enterprise tier features (`tier=3`) — included or deferred? | User | Deferred to v1.1 | M5 |

---

## 31. Acceptance Criteria for v1 GA

A v1 GA claim is valid only if **every** item below is satisfied with linkable evidence:

1. All 23 outcome-preservation rows in §2 demonstrably pass.
2. All 14 quality gates in §23 pass.
3. Every Critical and High inherited finding (C1, H1–H4) has a passing regression test that fails when the fix is reverted.
4. All 86 panels render real data on all 5 variants without visual regression > 0.5%.
5. Bootstrap (cold cache, 112 keys) ≤ 800 ms p95 from edge, ≤ 200 ms p95 from sidecar.
6. Health endpoint returns HEALTHY in production-like environment for ≥ 60 s with no STALE_SEED for non-cascade keys.
7. The full evidence bundle required by CLAUDE.md is produced: exact test command + output tail, `git diff --stat`, typecheck/lint output, explicit list of any deferred items.
8. No `RELAY_SHARED_SECRET`-bypass code path exists (grep clean for `if (!RELAY_SHARED_SECRET) return true` and equivalents).
9. Token rotation verified by sniffing IPC traffic over a 30-minute desktop session; ≥ 5 distinct tokens observed.
10. CSP triplication parity test passes; manual divergence is impossible (single source).

---

*End of SPEC-001.*
