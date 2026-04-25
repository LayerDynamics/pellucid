# WorldMonitor — Deep Investigation + Code Review

> Produced by `/lore:deep-investigation` + `/lore:code-review-methodology` against commit HEAD of `main` on 2026-04-17.
> Every claim is cited to `file:line`. Five parallel Explore agents plus direct ground-truth reads of `middleware.ts`, `src-tauri/src/main.rs`, `scripts/ais-relay.cjs`, `convex/http.ts`.

---

## PART 1 — HOW IT WORKS

### 1.1 System Topology

```
         ┌────────────────── Browser / Desktop (Tauri) ──────────────────┐
         │  src/main.ts ─► App.ts (8-phase init)                          │
         │   ├─ DeckGLMap (WebGL/deck.gl)  ├─ GlobeMap (three.js)         │
         │   ├─ 95 Panel subclasses        ├─ analysis + ml workers       │
         │   └─ AppContext (central mutable) + URL state (debounced 250ms)│
         └───────────────────────────── fetch /api/* ────────────────────┘
                │                     │                     │
      ┌─────────┼─────────┐  ┌────────┼────────┐   ┌───────┼────────┐
      │ Vercel Edge       │  │ Tauri Sidecar   │   │ Convex Cloud   │
      │ api/ + server/    │  │ local-api-server│   │ Clerk + Dodo   │
      │ gateway (14 stages│  │ :46123 dyn port │   │ schema, http   │
      └────┬──────────────┘  └────┬────────────┘   └────┬───────────┘
           │           ┌──────────┴────┐                │
           │           │ globalThis    │                │
           │           │ .fetch IPv4-4 │                │
           │           └─► relay + upstream             │
           ▼                         ▼                  ▼
   Upstash Redis ◄──── Railway Relay (scripts/ais-relay.cjs :3004)
   (+seed-meta)         │ AIS stream, OpenSky OAuth2, RSS proxy,
                        │ Telegram MTProto, OREF, ~20 seed loops
                        ▼
                    30+ upstream APIs
```

**Deployment inventory** (`ARCHITECTURE.md:56–68`): Vercel SPA+Edge, Railway relay+seeds, Upstash Redis, Convex Cloud, Mintlify docs, Tauri desktop, GHCR multi-arch Docker.

---

### 1.2 Primary Execution Paths

#### Path A — Browser page load (cold)

1. `src/main.ts:1-678` — Sentry + Vercel analytics + meta tags + `installRuntimeFetchPatch()` + `installWebApiRedirect()` + `loadDesktopSecrets()` + theme + variant detect (`src/config/variant.ts:1-32`: `VITE_VARIANT` → hostname `tech./finance./commodity./happy.` → localStorage on desktop).
2. `src/App.ts:791-1063` `async init()` 8 phases:
   - **P1** storage+i18n+`mlWorker.init()` + AIS stream + `waitForSidecarReady(3000)` on desktop
   - **P2** `fetchBootstrapData()` two-tier (`/api/bootstrap?tier=fast` 3s + `?tier=slow` 5s, separate `AbortController`s) + Clerk OAuth OTT verify + free-tier limits
   - **P3** Clerk auth subscribe, rebind Convex watches on sign-in, claim anon subs
   - **P4** `panelLayout.init()` → map + 95 panels consume pre-hydrated data; `SignalModal` + `IntelligenceGapBadge` + `BreakingNewsBanner` + `CorrelationEngine` (4 adapters: military, escalation, economic, disaster)
   - **P5** `searchManager.init()` + `countryIntel.init()` + `setupUrlStateSync()` (debounced via `src/utils/urlState.ts`)
   - **P6** parallel `loadAllData(true)` + viewport-conditional `primeVisiblePanelData(true)` (`panel.isNearViewport(400px)`) + initial `correlationEngine.run(state)`
   - **P7** `startSmartPollLoop()` (`src/app/refresh-scheduler.ts:44-76`): `pauseWhenHidden`, `maxBackoffMultiplier:4`, staggered flush (100ms first 4, 300ms rest) on tab visibility
   - **P8** `desktopUpdater.init()` (Tauri only)

#### Path B — `/api/<domain>/<rpc>` (edge)

Gateway pipeline (`server/gateway.ts:253-509`):

| # | Stage | file:line | Failure |
|---|---|---|---|
| 1 | Origin allowlist | gateway.ts:264 | 403 |
| 2 | CORS merge | gateway.ts:272 | — |
| 3 | OPTIONS preflight | gateway.ts:279 | 204 |
| 4 | Tier gate lookup | gateway.ts:285 | — |
| 5 | Clerk session (if gated) | gateway.ts:291 via `server/_shared/auth-session.ts:24-49` `jwtVerify` w/ JWKS | null if invalid |
| 6 | API key validate | gateway.ts:308 via `api/_api-key.js:34-70` | 401 |
| 7 | Entitlement check | gateway.ts:360 via `server/_shared/entitlement-check.ts:99-213` | 403 |
| 8 | Endpoint rate limit | gateway.ts:369 (`classify-event` 600/60s, `summarize-article-cache` 3000/60s) | 429 |
| 9 | Global rate limit | gateway.ts:372 (600/60s per IP, sliding window via `@upstash/ratelimit`) | 429 |
| 10 | Router match | `server/router.ts:30-78` (static Map + dynamic `{param}` scan; POST→GET compat <1MB) | 404/405 |
| 11 | Handler + error boundary | gateway.ts:413 | 500 |
| 12 | Header merge | gateway.ts:424 | — |
| 13 | FNV-1a ETag + 304 | gateway.ts:474-486 | 304 |
| 14 | Cache-Control + CDN-Cache-Control | gateway.ts:450 | — |

Handlers live at `server/worldmonitor/<domain>/v1/<rpc>.ts`. Examples:

- `aviation/v1/get-flight-status.ts:62,84` — key `aviation:status:${flight}:${date}:${origin}:v1`, upstream via Railway relay → aviationstack, tier `fast` (300s).
- `market/v1/list-market-quotes.ts:15,24` — hardcoded key `market:stocks-bootstrap:v1`, seeded (no upstream call), filter-on-read, tier `medium` (600s).
- `climate/v1/list-climate-anomalies.ts:14,21` — hardcoded key, seeded, tier `daily` (86400s).

#### Path C — Bootstrap hydration (batch read)

`api/bootstrap.js:210-273`: query-param `tier=fast|slow` or `keys=a,b` → `getCachedJsonBatch()` (single Upstash pipeline). 67 fast keys, 45 slow keys, 112 total (`BOOTSTRAP_CACHE_KEYS`). Fail-open: returns `{ data: {}, missing: [...] }` on Redis failure — no 5xx.

#### Path D — Health check

`api/health.js:541-685`: STRLEN all data keys + GET all `seed-meta:<key>` via pipeline. Classifies: OK / OK_CASCADE / STALE_SEED / SEED_ERROR / EMPTY / EMPTY_ON_DEMAND / REDIS_PARTIAL. Cascade groups (e.g., theater-posture: live/stale/backup) tolerate single-slot misses. Roll-up: HEALTHY / WARNING / DEGRADED (≤3% crit) / UNHEALTHY.

#### Path E — Railway relay (seed + proxy loop)

`scripts/ais-relay.cjs`:

- Server: raw `http` + `ws` on PORT (default 3004)
- **AIS**: `wss://stream.aisstream.io/v0/stream` with API key, exp backoff reconnect, HIGH/LOW watermark `upstreamQueue[]` (lines 48-58, 7100-7200)
- **OpenSky**: OAuth2 client_credentials, `openskyToken` cached w/ 60s buffer, mutex `openskyTokenPromise` (lines 7626-7750); positive cache 60s/128 entries + negative cache 30s + in-flight dedup
- **RSS**: allowlist from `shared/rss-allowed-domains.cjs`, 5min positive / 1min negative cache, in-flight dedup
- **Telegram**: MTProto StringSession, `data/telegram-channels.json`, gated by `TELEGRAM_CHANNEL_SET`, 60s poll / 15s per-channel timeout
- **OREF**: curl (JA3 bypass) + residential proxy fallback; history in Redis `relay:oref:history:v1`
- ~20 seed loops (market 5min, aviation 30min, NOTAM 2h, cyber 2h, positive 15min, theater-posture 5min, UCDP 30min, corridor-risk 1h, shipping-stress 1h, satellite-TLEs 2h, worldbank daily, etc.) — all start at `scripts/ais-relay.cjs:10783-10818` in `server.listen()` callback
- Auth: `x-relay-key` header or `Authorization: Bearer` → constant-time `safeTokenEquals` at line 6430 via `crypto.timingSafeEqual`

#### Path F — Desktop `/api/*` call

1. Renderer fetch → `src/services/runtime.ts:173-200` `toApiUrl()`
2. Desktop detected (`__TAURI__` global / `tauri://` protocol / `tauri.localhost` host) → rewrite to `http://127.0.0.1:{port}/api/...`
3. Port resolved via Tauri IPC `get_local_api_port` (`runtime.ts:30-49`) w/ retry; fallback 46123
4. Token via Tauri IPC `get_local_api_token` (`main.rs:242-252`, trusted-window gate) → `Authorization: Bearer <token>` injected
5. Sidecar `local-api-server.mjs` validates token, dynamically imports handler module via `buildRouteTable()`/`matchRoute()`/`importHandler()` (lines 296-519), invokes it with a synthesized `Request`
6. If handler 404/5xx and `LOCAL_API_CLOUD_FALLBACK=true` (set by Tauri at main.rs:1116) → `tryCloudFallback()` (lines 570-594) proxies to `https://api.worldmonitor.app`
7. Some paths are cloud-forced (lines 444-461: market/v1, economic/v1, bootstrap if no `WS_RELAY_URL`)

---

### 1.3 Cross-Service Boundaries (verified)

| From → To | Method + URL | Auth | Fail | Evidence |
|---|---|---|---|---|
| SPA → Edge | fetch `/api/*` | API key optional for trusted browser origin; Clerk Bearer on tier-gated | 401/403 | api/_cors.js:1-13,_api-key.js:34-70 |
| Desktop → Sidecar | fetch `http://127.0.0.1:{port}/api/*` | Bearer token from `get_local_api_token` IPC | 401 | main.rs:242-252, runtime.ts:30-49 |
| Sidecar → Cloud API | fetch `https://api.worldmonitor.app/api/*` (IPv4 forced) | origin/UA only | varies | local-api-server.mjs:570-594 |
| Edge → Relay | fetch `$WS_RELAY_URL/...` | `x-relay-key: $RELAY_SHARED_SECRET` | 403 | api/_relay.js:13-24 |
| Edge → Convex (checkout) | POST `${CONVEX_SITE_URL}/relay/create-checkout` | `Authorization: Bearer ${RELAY_SHARED_SECRET}` + timingSafeEqualStrings | 401 | convex/http.ts:646-712 |
| Edge → Convex (entitlements fallback) | POST `${CONVEX_SITE_URL}/api/internal-entitlements` | `x-convex-shared-secret: $CONVEX_SERVER_SHARED_SECRET` | 401 | convex/http.ts:57-96, entitlement-check.ts:131-139 |
| Dodo → Convex | POST `${CONVEX_SITE_URL}/dodopayments-webhook` | `webhook-id/-timestamp/-signature` HMAC-SHA256 via `verifyWebhookPayload` | 401/400 | convex/http.ts:637-641, webhookHandlers.ts:17-48 |
| Relay → aisstream | wss | `AISSTREAM_API_KEY` in connection | reconnect | ais-relay.cjs:436-7508 |
| Relay → OpenSky | HTTPS | OAuth2 client_credentials w/ 60s buffer + mutex | 90s cooldown on 429 | ais-relay.cjs:7626-7750 |

---

### 1.4 Caching Hierarchy

Four layers (`ARCHITECTURE.md §9`):

```
Railway seed ──► Upstash Redis (canonical)
                  │
      miss       ├── cachedFetchJson (stampede-protected, in-flight Map) ──► upstream
                  │    negative-cache sentinel on null (120s default)
      miss       └── returns 502/503 if fail-closed
```

Six tiers (`gateway.ts:34-42`):

| Tier | s-maxage | SWR | SIE | Use |
|---|---|---|---|---|
| fast | 300 | 60 | 1200 | aviation status, OREF, air quality |
| medium | 600 | 120 | 1800 | market quotes, crypto, hyperliquid |
| slow | 1800 | 300 | 7200 | ACLED, cyber threats, climate news |
| slow-browser | 900 | 60 | 1800 | premium supply-chain |
| static | 3600 | 600 | 28800 | ETF flows, airport delays |
| daily | 86400 | 3600 | 172800 | critical minerals, tariffs |
| no-store | 0 | — | — | vessels, aircraft tracking |

Premium paths force `slow-browser` override (`gateway.ts:451`). Env override `CACHE_TIER_OVERRIDE_${RPC_NAME}` per-RPC (`gateway.ts:449`).

**Seed atomic publish** (`scripts/_seed-utils.mjs:170-210`): validate → envelope `{_seed:{fetchedAt,recordCount,sourceVersion,state,...}, data}` (if opt-in) → serialize → 5MB size check → SET staging `{key}:staging:{runId}` TTL 5min → SET canonical `{key}` w/ TTL → DEL staging → SET `seed-meta:{domain}:{resource}` TTL max(7d, dataTtl). Lock via `SET seed-lock:{domain} {runId} NX PX {ttlMs}`, release with Lua compare-and-del.

---

### 1.5 Auth + Entitlement Flow

```
Browser
  │ Clerk token (template="convex" with plan claim)
  ▼
Edge gateway (tier-gated path only)
  │ jwtVerify → payload.sub = userId → x-user-id header
  ▼
entitlement-check.ts
  │ 1. Redis entitlements:${ENV_PREFIX}:${userId} (15min TTL, validUntil check)
  │ 2. miss → POST Convex /api/internal-entitlements (shared secret)
  │ 3. compare ent.features.tier >= ENDPOINT_ENTITLEMENTS[pathname]
  ▼
handler (or 403)
```

Tier map (`entitlement-check.ts:44-49`): 4 endpoints (market analyze-stock, get-stock-analysis-history, backtest-stock, list-stored-stock-backtests) require tier ≥ 2.

Plan catalog (`convex/config/productCatalog.ts`): free(0) → pro_monthly/annual(1) → api_starter(2) → api_business(2) → enterprise(3). Features: `{tier, maxDashboards, apiAccess, apiRateLimit, prioritySupport, exportFormats}`.

Legacy path (`gateway.ts:312-358`): `PREMIUM_RPC_PATHS` (33 endpoints) checks Bearer `role='pro'` — coexists with tier-based system.

Checkout identity: `DODO_IDENTITY_SIGNING_SECRET` HMAC-signs userId into Dodo checkout `metadata.wm_user_id_sig` (`convex/lib/identitySigning.ts:29-50`) — webhook verifies it (`subscriptionHelpers.ts:168-200`) to prevent client spoofing.

---

### 1.6 Variant System

Detection (`src/config/variant.ts:1-32`): build-time `VITE_VARIANT` → hostname prefix → desktop localStorage. Configs at `src/config/variants/{base,tech,finance,happy,commodity}.ts`. Variant change (`src/App.ts:424-449`) resets mapLayers, disables cross-variant panels, seeds defaults. Migration keys: `PANEL_KEY_RENAMES_MIGRATION_KEY`, `UNIFIED_MIGRATION_KEY`, `HAPPY_PANEL_FIX_KEY`. Social OG metadata duplicated in `middleware.ts:30-55` (served to crawlers on `/`).

---

## PART 2 — REVIEW FINDINGS (scored)

### Critical

**C1. Relay silently allows all traffic when `RELAY_SHARED_SECRET` is unset**

- Location: `scripts/ais-relay.cjs:6444-6449`

  ```js
  function isAuthorizedRequest(req) {
    if (!RELAY_SHARED_SECRET) return true;   // ← bypass
    const provided = getRelaySecretFromRequest(req);
    if (!provided) return false;
    return safeTokenEquals(provided, RELAY_SHARED_SECRET);
  }
  ```

- Impact: A Railway deploy that loses the `RELAY_SHARED_SECRET` env var (typo, rotation mistake, fresh env) becomes an open proxy for OpenSky quota, AIS stream, RSS proxy, and every seed endpoint. The relay runs on a public Railway hostname. `ALLOW_UNAUTHENTICATED_RELAY` (line 141-146) is explicitly for dev, but the code path above fires *regardless* of that flag when the secret string is empty.
- Why the `.env.example:215` comment "Must be set to the SAME value on both platforms in production" does not save you: this is documentation, not enforcement.
- Fix direction: hard-fail startup in production when secret is missing; require `ALLOW_UNAUTHENTICATED_RELAY=true` as the **only** way to bypass, and make it refuse to set alongside a production hostname env var.

### High

**H1. Tauri sidecar IPC token has no TTL — contradicts documented security model**

- Location: `src-tauri/src/main.rs:228-232` (generate once), stored in `LocalApiState::token` for the session. No rotation code exists.
- Contradiction: `ARCHITECTURE.md:238` states "Bearer <token> (5-min TTL from Tauri IPC)" and the skill's own brief repeats this. The code does not rotate.
- Impact: If any renderer-side XSS or chain-load bypass occurs (the CSP at `tauri.conf.json` allows `'unsafe-inline'` for styles, `'wasm-unsafe-eval'` and `https://*.clerk.accounts.dev` scripts), the leaked token is valid for the entire session instead of 5 minutes. Keyring-sourced API keys for 28 services (line 30-59) flow through that sidecar.
- Fix direction: either document the actual model (session-long + trusted-window gate + keychain-sourced secrets = acceptable risk) OR implement the rotation the docs claim.

**H2. Entitlement check fails closed on Convex outage — paying users get 403**

- Location: `server/_shared/entitlement-check.ts:99-213`. Flow: Redis miss → Convex fallback → both fail → return 403.
- Impact: Convex Cloud outage (or `CONVEX_SERVER_SHARED_SECRET` misconfiguration) blocks every Pro user from stock analysis, backtesting, and the other tier-2 endpoints. Redis cache is 15min (line 76) so a graceful decay exists for warm entitlements, but cold users with valid subscriptions see outage.
- Tradeoff: fail-closed is the correct default for entitlement gating *security-wise*. The review flag is on observability/UX: the handler does not distinguish "Convex unavailable" from "user genuinely unauthorized" — both return generic 403.
- Fix direction: return a distinct 503 with `Retry-After` when the upstream dependency fails (Redis + Convex both unreachable), so the client can show an outage banner rather than an "upgrade" prompt.

**H3. Startup circular dependency between relay seed loops and relay HTTP**

- Location: `scripts/ais-relay.cjs:4115-4302` (theater-posture seed) fetches `http://localhost:${PORT}/opensky` *against its own process*. Mitigated only by a 30s startup delay at line 4337.
- Impact: Port binding races or cold-start bursts (k8s restart, Railway cold boot) produce a 30s window where the seed misses its data source, silently logs a transient error, and awaits the next cycle.
- Fix direction: invoke the OpenSky fetch function directly (in-process) instead of round-tripping through the HTTP server.

**H4. Dual gating path increases attack surface for premium gating**

- Location: `server/gateway.ts:285-366`. Premium is gated two ways — legacy `PREMIUM_RPC_PATHS` (33 paths) checking Bearer `role='pro'` vs new `ENDPOINT_ENTITLEMENTS` (4 paths) checking `features.tier ≥ N`.
- Impact: A new premium endpoint added only to the legacy set will miss Redis caching and entitlement expiry checks; added only to the new set misses the Bearer role fallback. Migration path is undocumented.
- Fix direction: one-shot migration plan + tombstone `PREMIUM_RPC_PATHS`, OR make `ENDPOINT_ENTITLEMENTS` a strict superset with legacy paths auto-generated.

### Medium

**M1. Cache-key inconsistency — some RPCs include request params, some hardcode**

- `aviation/v1/get-flight-status.ts:62` correctly includes `${flightNumber}:${date}:${origin}`.
- `market/v1/list-market-quotes.ts:15` uses a hardcoded key and filters-on-read with `req.symbols`. This is intentional for seeded data, but the pattern inconsistency means future contributors can easily introduce cross-request leakage if they forget to include varying params. `ARCHITECTURE.md:297` calls this out as a MUST but it is not enforced.
- Fix: lint rule or test that scans `server/worldmonitor/**/v1/*.ts` and requires either (a) no request-body/query fields *or* (b) those fields appear in the key string.

**M2. Sidecar IPv4-only fetch monkey-patch is global**

- `local-api-server.mjs:112` sets `family: 4` on `https.request`. Justified for US government APIs with broken IPv6, but affects 60+ handler modules indiscriminately. No per-handler opt-out.
- Fix: per-module allowlist OR revert to dual-stack with explicit IPv4-fallback on specific hostnames.

**M3. Seeder silent-skip on transient Redis errors**

- `scripts/_seed-utils.mjs:130-136` `isTransientRedisError()` + seeder retry masks repeated outages. No alerting hook.
- Fix: emit a counter to the `/metrics` endpoint so repeated skips become visible.

**M4. Bootstrap fail-open returns partial data**

- `api/bootstrap.js:242`: Redis failure → `{ data: {}, missing: [...] }` with 200 status. Clients that don't inspect `missing[]` render empty panels as if the data were legitimately empty.
- Contrast: `api/health.js:580` returns `REDIS_DOWN` status.
- Fix: 503 + `Retry-After` when `missing.length === requested.length` (all-miss).

**M5. CSP triplicated across three files, no sync check**

- `index.html` meta + `vercel.json:80` header + `src-tauri/tauri.conf.json` CSP object. `ARCHITECTURE.md:253-260` documents the triplication explicitly but no CI check enforces parity. Manual drift guaranteed over time.
- Fix: single-source CSP builder script invoked by pre-push, writes all three.

**M6. Envelope migration incomplete — dual code paths**

- `api/bootstrap.js` and `api/health.js` both import `unwrapEnvelope`, but most seeders still write bare payloads (`scripts/_seed-utils.mjs:190` envelope is opt-in via `envelopeMeta`). Two shapes coexist indefinitely. New seeders can forget either direction.
- Fix: timeline to flip default + remove bare path.

**M7. OpenSky positive-cache LRU size 128 may thrash**

- `scripts/ais-relay.cjs:7515`: cap 128 entries for positive cache, 60s TTL. Map-heavy vessel dashboards querying many bboxes can churn; the negative cache sentinel at 30s partially compensates. Worth measuring hit rate at `/metrics`.

**M8. Rate limit prefixes are disjoint — no aggregate cap**

- `server/_shared/rate-limit.ts:17,101`: global uses `'rl'` prefix (600/60s), endpoint-specific uses `'rl:ep'` (separate counter). An IP hitting a high-limit endpoint (summarize-article-cache 3000/60s) can consume 3000 + 600 requests/min instead of being aggregated.
- Fix: add an umbrella cap or document the intended behavior.

**M9. Vault consolidation is load-once, no keychain change detection**

- `src-tauri/src/main.rs:1379` loads `SecretsCache` from keychain on startup. If the user rotates a key via `Keychain Access.app` while the app is running, the sidecar keeps injecting the stale value.
- Fix: subscribe to macOS keychain events OR add a "refresh secrets" IPC command + retry failed handlers.

### Low

**L1. Heavy keyring prompts on first launch** — consolidated vault (`main.rs:101-152`) migrates from individual entries, but the migration itself triggers N prompts. Fine for existing users, rough for first-time installs in migration window.

**L2. Commented-out migration code lingers** — `PANEL_KEY_RENAMES_MIGRATION_KEY` and `HAPPY_PANEL_FIX_KEY` one-time gates accrete with each rename. Prune after all released client versions pass the gate.

**L3. Relay `/health` endpoint apparently missing** — `Dockerfile.relay` healthcheck at line 39 runs `wget -qO- http://localhost:3004/health` but Explore agent could not locate the handler. Either the healthcheck is a no-op (exit 0 from wget 404?) or the agent missed it. Worth verifying explicitly.

**L4. Commented CSP allowlist entries in `tauri.conf.json`** — multiple hardcoded variant domains (`https://worldmonitor.app`, `https://tech.worldmonitor.app`, etc.) in `frame-src`. Self-hoster fork would need to edit this list.

**L5. Blog site postinstall slows `npm install` measurably** — `package.json:21` `"postinstall": "cd blog-site && npm ci --prefer-offline"` adds ~4s + 365 packages. Fine, but not documented as expected.

---

### Strengths (what this codebase does well)

1. **Timing-safe secret comparisons everywhere I checked** — relay (`ais-relay.cjs:6430` `crypto.timingSafeEqual`), Convex relay route (`http.ts:655` `timingSafeEqualStrings`), entitlement HTTP action (`convex/http.ts:62`). No plain `===` on secrets surfaced in audit.
2. **Secret separation is clean** — four distinct secrets (`RELAY_SHARED_SECRET`, `CONVEX_SERVER_SHARED_SECRET`, `DODO_PAYMENTS_WEBHOOK_SECRET`, `DODO_IDENTITY_SIGNING_SECRET`). `.env.example:314-317` explicitly warns not to reuse values.
3. **Stampede-protected Redis cache** — `server/_shared/redis.ts:198,215-216` in-flight Map coalesces concurrent misses into a single upstream fetch + single Redis write. Negative-cache sentinel (`NEG_SENTINEL`) prevents null-result storms.
4. **Consistent 14-stage gateway pipeline** — cleanly ordered, each stage is a discrete function, failure modes are explicit.
5. **Static+dynamic route matching w/ POST→GET compat** — `server/router.ts:30-78` gives O(1) on static routes with graceful fallback to a sorted-length dynamic scan, plus body-size-checked POST→GET for stale clients.
6. **Webhook idempotency keyed by `webhook-id`** — `convex/payments/webhookMutations.ts:33-46` deduplicates; failed mutations trigger 500 so Dodo retries until consistent.
7. **HMAC-signed userId in checkout metadata** — prevents client-side spoofing of Pro-plan assignment; constant-time verify at `convex/lib/identitySigning.ts:56-72`.
8. **Rich test surface** — 60+ `tests/*.test.{mjs,mts}` covering auth sessions, bootstrap, cache invariants, circuit breakers, seeders, CII scoring, clustering, country geometry, chokepoints, etc. Plus 12 Playwright e2e specs with variant-scoped golden screenshots.
9. **Pre-push hook is substantive** — tsc src + api, CJS syntax, edge bundle esbuild, edge-function import guardrail test, markdown+MDX lint, version sync. Shipping a broken build intentionally takes effort.
10. **Vault consolidation** — single keychain entry `secrets-vault` (`main.rs:101-152`) avoids repeated per-key Touch ID prompts. Migration from legacy individual entries is transactional.
11. **Client-side circuit breakers** — `src/utils/circuit-breaker.ts` used by `src/app/data-loader.ts` prevents cascade failures when a single upstream flaps.
12. **Proto/sebuf contract enforcement in CI** — `.github/workflows/proto-check.yml` fails PRs where generated client stubs drift from proto definitions. Keeps OpenAPI + RPC client + server in lockstep.
13. **Cache-tier explicit env override** — `CACHE_TIER_OVERRIDE_${RPC_NAME}` (`gateway.ts:449`) lets ops escalate/de-escalate caching without a deploy.

---

### Open questions (requires human judgment)

1. Is the documented 5-min sidecar token TTL a future goal, or should the docs be corrected?
2. Migration timeline for legacy `PREMIUM_RPC_PATHS` → tier-based `ENDPOINT_ENTITLEMENTS`?
3. Entitlement fail-closed vs 503-on-upstream-down — intentional product decision or accident?
4. How does the SPA surface premium gating to anon/free users client-side? Agent D couldn't locate the consumer; appears to be `role` from Clerk rather than Convex entitlements.
5. Relay `/health` endpoint presence — Dockerfile healthcheck looks aspirational.
