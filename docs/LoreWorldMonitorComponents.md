# World Monitor — Code-Level Component Breakdown

All citations verified against the current repository state (branch `main`).

## 1. Entry & Bootstrap

### `src/main.ts` (677 lines)

Boots the page: Sentry init, Vercel analytics, dynamic meta tags, runtime fetch patch install, theme application, creates `App` instance, wires chunk-reload recovery.

### `src/App.ts` (1,486 lines)

- `class App` — owns the entire app lifecycle.
  - `public async init(): Promise<void>` (L791) — the 8-phase boot sequence.
  - `public destroy(): void` (L1115) — tears down listeners, intervals, workers.
- Imports from `@/app`, `@/config`, `@/services`, `@/components`, `@/utils`.

### `src/app/app-context.ts` (95 lines)

- `interface AppContext` / `createAppContext()` — central mutable state object (maps, panel instances, caches, in-flight trackers, UI refs).

### `src/bootstrap/` + `src/services/bootstrap.ts` (190 lines)

- `getHydratedData(key)` (L32), `markBootstrapAsLive()` (L38), `getBootstrapHydrationState()` (L55), `fetchBootstrapData()` (L157) — two-tier (fast/slow) concurrent hydration from `/api/bootstrap`.

---

## 2. Orchestration (`src/app/`)

### `data-loader.ts` (3,356 lines)

- `class DataLoaderManager implements AppModule` (L255).
  - `init()` / `destroy()` (L300 / L320)
  - `loadAllData(forceAll)` (L422) — master loader
  - Per-domain loaders: `loadDataForLayer(layer)` (L637), `loadSatellites` (L720), `loadNews` (L1042), `loadStockAnalysis` (L1209), `loadStockBacktest` (L1316), `loadMarkets` (L1352), `loadDailyMarketBrief` (L1582), `loadMarketImplications` (L1739), `loadPredictions` (L1761), plus `loadNewsCategory`, `loadImageryFootprints`, `refreshTemporalBaseline`, etc.
  - Viewport / time helpers: `isPanelNearViewport` (L413), `getTimeRangeWindowMs` (L825), `filterItemsByTimeRange` (L837), `applyTimeRangeFilterDebounced` (L876).
  - Map feedback: `findFlashLocation` (L766), `flashMapForNews` (L799).

### `refresh-scheduler.ts` (119 lines)

- `class RefreshScheduler implements AppModule`:
  - `setHiddenSince` / `getHiddenSince` (L36 / L40) — tab-visibility state.
  - `scheduleRefresh(...)` (L44) — viewport-conditional + exponential-backoff polling.
  - `flushStaleRefreshes()` (L78) — staggered 150 ms flush on visibility change.
  - `registerAll(registrations)` (L114) — bulk registration.

### `panel-layout.ts` (2,142 lines)

- Panel grid layout manager: renders the panel DOM, handles row/col span persistence to localStorage, wires resize handles, integrates with variant config.

### Other `src/app/` modules

- `event-handlers.ts`, `search-manager.ts`, `country-intel.ts`, `pending-panel-data.ts`, `desktop-updater.ts`, `index.ts`.

---

## 3. UI Components (`src/components/` — ~140 files)

### `Panel.ts` (1,203 lines) — base class for all 86 panels

- `export class Panel` (L199):
  - Constructor + lifecycle: `constructor(options)` (L244), `getElement()` (L751), `hide` / `show` / `toggle` (L1044–L1052).
  - Rendering: `setContent(html)` (L1011, 150 ms-debounced) → `setContentImmediate` (L1032).
  - State views: `showLoading` (L776), `showError` (L791), `showLocked` (L831), `showGatedCta` (L871), `unlockPanel` (L908), `showRetrying` (L920), `showConfigError` (L976).
  - Resize: `restoreSavedColSpan` (L372), `reconcileColSpanAfterAttach` (L391), `setupResizeHandlers` (L434), `setupColResizeHandlers` (L574), touch listener pairs (L410 / L422, L550 / L562).
  - Badges / severity: `setDataBadge` (L680), `insertLiveCountBadge` (L697), `setCount` (L990), `setNewBadge` / `clearNewBadge` (L1062 / L1086), `setSeverity` (L1094), `setErrorState` (L1002).
  - Header controls: `appendCollapseButton` (L719), `appendCloseButton` (L735).
  - Viewport check: `isNearViewport(marginPx)` (L755).

### Map components

- `DeckGLMap.ts` (6,607 lines) — deck.gl + maplibre-gl: Scatterplot / GeoJson / Path / Icon / Polygon / Arc / Heatmap / H3Hexagon layer management, PMTiles protocol, supercluster clustering.
- `GlobeMap.ts` (3,578 lines) — globe.gl 3D: merged `htmlElementsData` with `_kind` discriminator, atmosphere shader, auto-rotate.
- `MapContainer.ts`, `MapPopup.ts`, `MapContextMenu.ts`, `PlaybackControl.ts` — map chrome.

### Panel families (grouped, all extend `Panel`)

- News / intel: `NewsPanel`, `LiveNewsPanel`, `BreakingNewsBanner`, `GdeltIntelPanel`, `TelegramIntelPanel`, `RegionalIntelligenceBoard`, `CountryDeepDivePanel`, `CountryBriefPanel`.
- Markets / finance: `MarketPanel`, `StockAnalysisPanel`, `StockBacktestPanel`, `MarketBreadthPanel`, `ETFFlowsPanel`, `FearGreedPanel`, `CotPositioningPanel`, `EarningsCalendarPanel`, `YieldCurvePanel`, `StablecoinPanel`, `LiquidityShiftsPanel`, `DailyMarketBriefPanel`.
- Macro / economy: `EconomicPanel`, `ConsumerPricesPanel`, `FSIPanel`, `MacroSignalsPanel`, `MacroTilesPanel`, `NationalDebtPanel`, `BigMacPanel`, `GroceryBasketPanel`, `FuelPricesPanel`, `FaoFoodPriceIndexPanel`, `GulfEconomiesPanel`.
- Energy / commodities: `EnergyComplexPanel`, `EnergyCrisisPanel`, `OilInventoriesPanel`, `HormuzPanel`, `RenewableEnergyPanel`, `GoldIntelligencePanel`.
- Geopolitics / military / conflict: `UcdpEventsPanel`, `StrategicPosturePanel`, `StrategicRiskPanel`, `MilitaryCorrelationPanel`, `EscalationCorrelationPanel`, `ThermalEscalationPanel`, `DefensePatentsPanel`, `SanctionsPressurePanel`, `SupplyChainPanel`, `TradePolicyPanel`.
- Climate / nature: `ClimateAnomalyPanel`, `ClimateNewsPanel`, `DisasterCorrelationPanel`, `SatelliteFiresPanel`, `RadiationWatchPanel`, `DiseaseOutbreaksPanel`, `SpeciesComebackPanel`.
- Infra / cyber: `InternetDisruptionsPanel`, `SecurityAdvisoriesPanel`, `ServiceStatusPanel`, `CIIPanel`, `CommunityWidget`.
- Forecast / prediction: `ForecastPanel`, `PredictionPanel`, `DeductionPanel`, `CrossSourceSignalsPanel`, `CorrelationPanel`.
- Chat / MCP: `ChatAnalystPanel`, `WidgetChatModal`, `McpConnectModal`, `McpDataPanel`.
- Modals / utilities: `SignalModal`, `StoryModal`, `SearchModal`, `CountryIntelModal`, `MobileWarningModal`, `UnifiedSettings`, `VirtualList`, `IntelligenceGapBadge`, `LlmStatusIndicator`.
- Auth / billing: `AuthHeaderWidget`, `AuthLauncher`, `ProBanner`, `DownloadBanner`, `payment-failure-banner`.

---

## 4. Services (`src/services/` — ~200 files)

Organized by domain; each file exports pure functions consumed by panels and loaders:

- **AI / ML**: `analysis-worker.ts`, `ai-classify-queue.ts`, `ai-flow-settings.ts`, `analysis-framework-store.ts`, `analysis-core.ts`, `correlation-engine/`, `correlation.ts`, `cross-source-signals.ts`, `entity-extraction.ts`, `entity-index.ts`, `clustering.ts`.
- **Bootstrap / runtime**: `bootstrap.ts`, `runtime.ts`, `desktop-readiness.ts`, `data-freshness.ts`, `meta-tags.ts`.
- **Market**: `market/`, `market-implications.ts`, `market-watchlist.ts`, `daily-market-brief.ts`, `insider-transactions.ts`.
- **Conflict / military**: `conflict/`, `military/`, `military-bases.ts`, `hormuz-tracker.ts`, `hotspot-escalation.ts`.
- **Climate / nature**: `climate/`, `climate-air-quality.ts`, `earthquakes.ts`, `eonet.ts`, `disease-outbreaks.ts`.
- **Infra / cable / maritime / aviation**: `maritime/`, `aviation/`, `infrastructure/`, `infrastructure-cascade.ts`, `cable-activity.ts`, `cable-health.ts`.
- **Auth / payment**: `clerk.ts`, `auth-state.ts`, `billing.ts`, `checkout.ts`, `checkout-return.ts`, `entitlements.ts`, `convex-client.ts`.
- **Domain libs**: `forecast.ts`, `gdelt-intel.ts`, `displacement/`, `giving/`, `cyber/`, `economic/`, `intelligence/`, `consumer-prices/`, `research/`, etc.
- `index.ts` re-exports the public service surface consumed by `App.ts` and panels.

---

## 5. Workers (`src/workers/`)

- `analysis.worker.ts` — Jaccard clustering + cross-domain correlation.
- `ml.worker.ts` — ONNX inference via `@xenova/transformers` (MiniLM-L6 embeddings, sentiment, summarization, NER).
- `vector-db.ts` — IndexedDB-backed vector store for semantic search.

---

## 6. Runtime & Desktop (`src/services/runtime.ts`, 910 lines)

- `resolveLocalApiPort()` (L30), `getLocalApiPort()` (L51) — Tauri IPC port discovery.
- `detectDesktopRuntime(probe)` (L67), `isDesktopRuntime()` (L92).
- URL builders: `getApiBaseUrl` (L110), `getConfiguredWebApiBaseUrl` (L129), `getCanonicalApiOrigin` (L150), `getRemoteApiBaseUrl` (L154), `toRuntimeUrl` (L173), `toApiUrl` (L186).
- `class VisibilityHub` (L284) — subscribable tab-visibility broker.
- `startSmartPollLoop(...)` (L342) — exponential backoff (max 4×), viewport-gated, tab-paused.
- `waitForSidecarReady(timeoutMs)` (L525), `installRuntimeFetchPatch()` (L605), `installWebApiRedirect()` (L749).

### `src-tauri/` (Rust + Node sidecar)

- `src-tauri/src/main.rs` — app lifecycle, tray, IPC commands (keyring r/w, sidecar spawn).
- `src-tauri/sidecar/local-api-server.mjs` (1,580 lines) — dynamic-port HTTP server that loads Edge handler modules from `api/`, injects keyring secrets as env, monkey-patches `globalThis.fetch` to force IPv4, issues 5-min bearer tokens.
- Configs: `tauri.conf.json`, `tauri.tech.conf.json`, `tauri.finance.conf.json`.

---

## 7. Edge API (`api/` — self-contained JS)

### Shared helpers (`api/_*.js`)

- `_cors.js` — origin allowlist + CORS header builder.
- `_rate-limit.js` / `_ip-rate-limit.js` — Upstash sliding-window, IP extraction.
- `_api-key.js` — origin-aware key validation; trusted-browser exemption, premium RPC gate.
- `_relay.js` — Railway relay proxy factory.
- `_crypto.js`, `_turnstile.js`, `_upstash-json.js`, `_json-response.js`, `_sentry-edge.js`, `_oauth-token.js`, `_email-validation.js`, `_seed-envelope.js`, `_rss-allowed-domains.js`, `_github-release.js`, `_product-fallback-prices.js`.

### Top-level endpoints

`bootstrap.js` (273 lines, batch Redis reads), `health.js` (685 lines, per-key freshness vs `seed-meta`), `ais-snapshot.js`, `cache-purge.js`, `contact.js`, `create-checkout.ts`, `download.js`, `geo.js`, `gpsjam.js`, `mcp.ts`, `mcp-proxy.js`, `military-flights.js`, `notify.ts`, `og-story.js`, `opensky.js`, `oref-alerts.js`, `polymarket.js`, `register-interest.js`, `reverse-geocode.js`, `rss-proxy.js`, `sanctions-entity-search.js`, `satellites.js`, `seed-contract-probe.ts`, `seed-health.js`, `story.js`, `telegram-feed.js`, `user-prefs.ts`, `widget-agent.ts`.

### Domain bundles (mirror proto services)

`aviation/`, `climate/`, `conflict/`, `consumer-prices/`, `cyber/`, `data/`, `discord/`, `displacement/`, `economic/`, `eia/`, `enrichment/`, `forecast/`, `giving/`, `health/`, `imagery/`, `infrastructure/`, `intelligence/`, `maritime/`, `market/`, `military/`, `natural/`, `news/`, `notification-channels.ts`, `oauth/`, `positive-events/`, `prediction/`, `radiation/`, `research/`, `resilience/`, `sanctions/`, `scenario/`, `seismology/`, `skills/`, `slack/`, `supply-chain/`, `telegram/`, `thermal/`, `trade/`, `unrest/`, `v2/`, `webcam/`, `wildfire/`, `youtube/`.

---

## 8. Server Gateway (`server/`)

### `server/gateway.ts` (509 lines)

- `serverOptions` (L23): `{ onError: mapErrorToResponse }` passed into sebuf-generated handlers.
- Cache-tier tables: `TIER_HEADERS` (L34), `TIER_CDN_CACHE` (L47), `RPC_CACHE_TIER` (L57+).
- `export function createDomainGateway(routes)` (L253) → returns `handler(Request) → Promise<Response>` pipeline:
  1. Path normalization + origin check (403 if disallowed).
  2. CORS header computation; OPTIONS preflight (204).
  3. Tier-gate detection (`getRequiredTier`), optional Clerk JWT resolve (`resolveSessionUserId`) and `x-user-id` header injection.
  4. API-key validation (`validateApiKey`) with bearer-token fallback for legacy pro RPCs (`validateBearerToken`).
  5. Entitlement check for tier-gated RPCs (`checkEntitlement`).
  6. Rate limiting: endpoint-specific (`checkEndpointRateLimit`) then global (`checkRateLimit`).
  7. Route matching (`createRouter` static Map → dynamic `{param}` scan).
  8. POST → GET compat.
  9. Handler execution inside error boundary (`mapErrorToResponse`).
  10. ETag (FNV-1a via `_shared/hash.ts`) + `If-None-Match` → 304.
  11. Cache header application via `drainResponseHeaders`.

### `server/router.ts` (108 lines) + `server/cors.ts` + `server/error-mapper.ts` + `server/auth-session.ts`

### `server/_shared/` (30+ files)

- `redis.ts` (366 lines): `getRawJson` (L40), `getCachedJson` (L60), `setCachedJson` (L88), `getCachedJsonBatch` (L117), `runRedisPipeline` (L163), `cachedFetchJson` (L205, stampede-coalesced single upstream fetch per key), `cachedFetchJsonWithMeta` (L248), `geoSearchByBox` (L286), `getHashFieldsBatch` (L314), `deleteRedisKey` (L351), `__resetKeyPrefixCacheForTests` (L31).
- `rate-limit.ts` (133 lines) — Upstash sliding window.
- `fetch-json.ts`, `cache-keys.ts`, `hash.ts`, `response-headers.ts`, `parse-string-array.ts`, `normalize-list.ts`, `timing-safe.ts`, `premium-check.ts`, `entitlement-check.ts`, `auth-session.ts`, `source-tiers.ts`, `sidecar-cache.ts`, `llm.ts`, `llm-sanitize.js`, `llm-health.ts`, `acled.ts`, `acled-auth.ts`, `airline-codes.ts`, `air-quality-stations.ts`, `bypass-corridors.ts`, `chokepoint-registry.ts`, `country-token.ts`, `constants.ts`, `relay.ts`, `resilience-freshness.ts`, `resilience-stats.ts`, `seed-envelope.ts`.

### `server/worldmonitor/<domain>/v1/handler.ts` (30+ domains)

Each exports a handler object with per-RPC async functions; each RPC uses `cachedFetchJson()` with a param-scoped cache key. Domains: `aviation`, `climate`, `conflict`, `consumer-prices`, `cyber`, `displacement`, `economic`, `forecast`, `giving`, `health`, `imagery`, `infrastructure`, `intelligence`, `maritime`, `market`, `military`, `natural`, `news`, `positive-events`, `prediction`, `radiation`, `research`, `resilience`, `sanctions`, `seismology`, `supply-chain`, `thermal`, `trade`, `unrest`, `webcam`, `wildfire`.

---

## 9. Proto / RPC (`proto/`)

- `buf.yaml`, `buf.gen.yaml`, `buf.lock`.
- `proto/sebuf/` — sebuf HTTP annotation extensions.
- `proto/worldmonitor/<domain>/` — service definitions with `(sebuf.http.config)` per RPC.
- `Makefile` → `make generate` → `src/generated/client/` (TS client stubs) + `src/generated/server/` (handler types) + `docs/api/` (OpenAPI v3).

---

## 10. Seed Pipeline (`scripts/`)

### `_seed-utils.mjs` (994 lines)

`atomicPublish()` — Redis `SET NX` lock → validate payload → write cache key → write `seed-meta:<key>` (`{ fetchedAt, recordCount }`) → release lock. Also: envelope builders, freshness math, chunked SET helpers.

### `_*.mjs` shared libraries

`_bundle-runner.mjs`, `_clustering.mjs`, `_climate-zones.mjs`, `_country-resolver.mjs`, `_digest-markdown.mjs`, `_ema-threat-engine.mjs`, `_eurostat-utils.mjs`, `_gdelt-fetch.mjs`, `_llm-json.mjs`, `_military-surges.mjs`, `_open-meteo-archive.mjs`, `_prediction-scoring.mjs`, `_proxy-utils.cjs`, `_r2-storage.mjs`, `_seed-contract.mjs`, `_seed-envelope-source.mjs`, `_shared-av.mjs`, `_ticker-validation.mjs`, `_trade-parse-utils.mjs`, `_yahoo-fetch.mjs`.

### `seed-*.mjs` (~140 scripts)

Domain seeders: market quotes (`seed-market-quotes`, `seed-commodity-quotes`, `seed-crypto-quotes`, `seed-gulf-quotes`), ETF / flows (`seed-etf-flows`, `seed-gold-etf-flows`), macro (`seed-bundle-macro`, `seed-imf-*`, `seed-ecb-*`, `seed-eurostat-*`, `seed-bls-series`), conflict / geopolitics (`seed-ucdp-events`, `seed-gdelt-intel`, `seed-conflict-intel`, `seed-iran-events`, `seed-unrest-events`), climate (`seed-climate-*`, `seed-fire-detections`, `seed-earthquakes`, `seed-natural-events`), energy (`seed-energy-*`, `seed-fuel-prices`, `seed-jodi-*`, `seed-spr-policies`, `seed-iea-oil-stocks`, `seed-gie-gas-storage`), resilience (`seed-bundle-resilience*`, `seed-resilience-scores`, `seed-resilience-intervals`, `seed-recovery-*`), trade / supply (`seed-trade-flows`, `seed-supply-chain-trade`, `seed-comtrade-bilateral-hs4`, `seed-portwatch*`, `seed-chokepoint-*`, `seed-hormuz`, `seed-submarine-cables`), health / humanitarian (`seed-health-air-quality`, `seed-disease-outbreaks`, `seed-displacement-summary`), cyber (`seed-cyber-threats`, `seed-security-advisories`, `seed-internet-outages`), forecasting / prediction (`seed-forecasts`, `seed-prediction-markets`, `seed-cot`, `seed-correlation`, `seed-cross-source-signals`). Each ends with an `atomicPublish()` call.

### Long-running services

- `ais-relay.cjs` (10,891 lines) — Railway WebSocket proxy + continuous seed loops (markets, aviation, GPSJAM, risk scores, UCDP, positive events, RSS proxy, OREF polling).
- `notification-relay.cjs` — push / alert relay.
- `scenario-worker.mjs`, `process-deep-forecast-tasks.mjs`, `process-simulation-tasks.mjs` — async task workers.
- Validation / benchmark: `validate-rss-feeds.mjs`, `validate-seed-migration.mjs`, `validate-resilience-*.mjs`, `verify-seed-envelope-parity.mjs`, `backtest-resilience-outcomes.mjs`, `benchmark-resilience-external.mjs`, `evaluate-forecast-*.mjs`, `diff-forecast-runs.mjs`, `replay-forecast-run.mjs`, `promote-forecast-benchmark-candidate.mjs`, `shadow-score-rank.mjs`, `shadow-score-report.mjs`.
- Build helpers: `build-sidecar-sebuf.mjs`, `build-sidecar-handlers.mjs`, `desktop-package.mjs`, `sync-desktop-version.mjs`, `lint-boundaries.mjs`, `check-unicode-safety.mjs`.

---

## 11. Edge Middleware (`middleware.ts`, 148 lines)

Vercel edge middleware: bot-UA filter on API / asset paths, social-preview UA allowlist (Twitter / Facebook / LinkedIn / Telegram / Discord) on story / OG paths, CSP bookkeeping.

---

## 12. Convex (`convex/`)

- `schema.ts` — contact submissions + waitlist.
- Queries / mutations for contact form and waitlist registrations; tested with `convex-test` under Vitest (`npm run test:convex`).

---

## 13. Tests

- `tests/*.test.{mjs,mts}` — node:test over handlers, cache keys, circuit breakers, edge constraints, dedup, health, panel / layer guardrails.
- `tests/edge-functions.test.mjs` — forbids `node:` builtins and cross-dir imports in `api/*.js`.
- `api/*.test.mjs` (`_cors.test.mjs`, `_turnstile.test.mjs`, `og-story.test.mjs`, `loaders-xml-wms-regression.test.mjs`, etc.).
- `src-tauri/sidecar/local-api-server.test.mjs`.
- `e2e/*.spec.ts` — Playwright per variant + visual-regression goldens.
- `server/__tests__/` — gateway / router unit tests.

---

## 14. Build / CI

- Build: `npm run build` (blog → tsc → vite), variant builds (`build:full`, `build:tech`, `build:finance`, `build:happy`, `build:commodity`), desktop (`build:desktop`), packaging (`desktop:package:*:sign`).
- Generate: `make generate` (buf + sebuf plugins).
- Workflows: `.github/workflows/typecheck.yml`, `lint.yml`, `proto-check.yml`, `build-desktop.yml`, `docker-publish.yml`, `test-linux-app.yml`.
- `.husky/pre-push`: typecheck (src + api), CJS syntax, edge esbuild bundle, edge import guardrail test, markdown lint, MDX lint, version sync.
