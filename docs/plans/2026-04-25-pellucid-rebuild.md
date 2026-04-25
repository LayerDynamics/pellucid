# 2026-04-25 — Pellucid Rebuild Implementation Plan

**Authoritative spec:** `docs/specs/SPEC-001-pellucid-stack-rebuild.md`
**Outcome:** GA of Pellucid (Tauri + Bun + Vite + React + Tailwind + Radix + Zustand + SQLite + Rust workspace) at functional parity with WorldMonitor over ~25 weeks.
**Plan structure:** Single master plan. Milestones M0–M6 follow spec §28. Hybrid M3 panel breakdown (9 family tasks each owning N panel sub-tasks). Per-task verification commands.
**Test mandate (user-locked):** Unit + integration + e2e tests for **every** public method/function/IPC command/RPC handler/component. No exceptions, no deferrals. CI gate enforces.

---

## 0. How to use this plan

1. **Read sequentially.** Tasks within a milestone are ordered by dependency. Skipping forward is allowed only if a task's `Deps:` line is satisfied.
2. **Track via Claude Code TaskList.** Each task has a `T<milestone>.<n>` ID. Mark `in_progress` before starting; `completed` only after all `Verify:` commands pass and CLAUDE.md evidence requirements are met.
3. **Resume points** for `/lore:execute`: every milestone exit gate is a checkpoint.
4. **Decision lock-ins** (locked during Phase 2 Q&A on this plan):
   - Plan layout: single master file
   - M3 granularity: hybrid (family task + per-panel sub-tasks)
   - Verification cadence: per-task commands (and milestone gate as superset)
   - Repo init: Task 0 does full bootstrap (git init, workspace, license, scripts, CI scaffold)
   - Test coverage: unit + integration + e2e for ALL methods
5. **CLAUDE.md compliance** (project + global) is non-negotiable:
   - Before claiming a task complete: produce (1) exact test command + output tail, (2) `git diff --stat`, (3) typecheck/lint output, (4) explicit deferred-items list.
   - Every bug/security fix ships with a regression test that **fails without the fix and passes with it**, verified by temporary revert.
   - No stubs, mocks, placeholders. No "TODO", "implement later". No fake data.
   - "Same outcome" mandate: every preserved behavior in spec §2 (OP-1 through OP-23) must demonstrably ship.

---

## 1. Universal Test Mandate

The user specified **unit + integration + e2e for ALL methods**. Every task in this plan must satisfy this mandate. No task may be marked complete without all three tiers passing for the code it adds or modifies.

### 1.1 Definitions

| Tier | Scope | Tools |
|---|---|---|
| **Unit** | Single function/method in isolation. No I/O, no external services, no network. Test files co-located with source: Rust `#[cfg(test)] mod tests` or `tests/<file>.rs`; TS `*.test.ts` next to source. | `cargo nextest run -p <crate>` for Rust; `bun test <path>` for TS |
| **Integration** | Multiple modules together OR module + its real dependency (real SQLite, real Axum router, real Tauri IPC bridge, real Convex test environment). Cross-crate, but in-process. Located in `crates/<crate>/tests/` (Rust) or `webview/integration/` (TS). | `cargo nextest run --test '*'` for Rust; `bun test webview/integration` for TS |
| **E2E** | Per CLAUDE.md: full user-facing workflow front-to-back through the **real** system. No mocked components across boundaries. Real webview → real Axum → real SQLite → real upstream sandbox (or VCR cassette where no sandbox exists). Located in `e2e/`. | `bunx playwright test` (web) or `bunx playwright test --project=desktop` |

### 1.2 Coverage requirement

- **Unit**: ≥ 90 % line coverage per crate / per webview module (`cargo llvm-cov`, `bun test --coverage`).
- **Integration**: every public API boundary (Tauri command, RPC handler, Convex action, IPC channel) must have at least one integration test exercising the real implementation.
- **E2E**: every user journey listed in spec §2 (OP-2 boot, OP-3 gateway, OP-4 bootstrap, OP-5 health, OP-6 stream, OP-7 desktop sidecar, OP-13 variant switch, OP-15 map render, OP-17 poll loop, OP-18 webhook idempotency) must have at least one Playwright spec.

### 1.3 CI gate (universal)

The `bun run check` task in the Justfile (added in T0.10) must run, in order:

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo nextest run --workspace
cargo llvm-cov --workspace --fail-under-lines 90
bun run typecheck
bun run lint
bun test --coverage
bun run check-cache-keys
bun run check-csp
bun run check-edge-imports
bunx playwright test
```

A pull request that fails any of these blocks merge.

### 1.4 Test-first discipline

Every task in this plan is **test-driven**: write the failing test, see it fail, implement the code, see it pass. The task's `Tests:` section is therefore written before the `Files:` section in implementation order.

---

## 2. Test Tooling Stack (locked here, used by every task)

| Concern | Tool | Version | Wired in task |
|---|---|---|---|
| Rust unit + integration runner | `cargo nextest` | latest stable | T0.5 |
| Rust coverage | `cargo llvm-cov` | latest stable | T0.5 |
| Rust property tests (where applicable) | `proptest` | 1.x | T0.5 |
| Rust mocking at boundaries (only for upstream APIs in unit tests; integration uses real services) | `wiremock` | 0.6 | T0.5 |
| TS unit + integration runner | `bun test` (built-in) | bun 1.x | T0.7 |
| TS component testing | `@testing-library/react` + `@testing-library/jest-dom` (with bun's jest-compatible API) | latest | T0.7 |
| Network cassettes for replay-tests of upstreams | `vcr-rs` (Rust) + `polly.js` (TS) | latest | T0.5 / T0.7 |
| E2E framework | Playwright | 1.x | T0.8 |
| Tauri E2E driver | `tauri-driver` + `@playwright/test` Tauri integration | latest | T0.8 |
| Visual regression | Playwright + `pixelmatch` | latest | T0.8 |
| Convex test harness | `convex-test` under `bun test` | latest | T0.7 |

---

## 3. Workstream legend

Each task header uses these tags:

- `[setup]` infrastructure, repo, build, CI
- `[rust]` Rust code in `crates/`
- `[webview]` TypeScript code in `webview/`
- `[convex]` Convex code in `convex/`
- `[fix]` regression-test-bearing fix for an inherited finding (C/H/M/L)
- `[panel]` one of the 86 panels (M3 only)
- `[gate]` milestone exit verification (no code; confirms the milestone is closeable)

---

## Task 0 — Repository Initialization

**Goal:** Convert `/Users/ryanoboyle/pellucid` (currently 6 empty directories + `CLAUDE.md` + `docs/`) into a fully bootstrapped polyglot monorepo.

### T0.1 — Initialize git repository [setup]
**Deps:** none
**Files:** `.gitignore`, `.gitattributes`, `.editorconfig`, `LICENSE`, `README.md`
**Deliverable:** `git init` + initial commit containing CLAUDE.md, docs/, and the new dotfiles.
**Tests:** none (configuration-only task; verified by gate)
**Verify:**

```bash
git -C /Users/ryanoboyle/pellucid log --oneline | head -1   # shows first commit
git -C /Users/ryanoboyle/pellucid status                    # clean working tree
test -f /Users/ryanoboyle/pellucid/.gitignore
```

`.gitignore` must include: `target/`, `node_modules/`, `.bun/`, `dist/`, `coverage/`, `*.db`, `*.db-shm`, `*.db-wal`, `.env`, `.env.local`, `.DS_Store`, `src-tauri/target/`, `playwright-report/`, `test-results/`.

### T0.2 — Top-level Cargo workspace [setup][rust]
**Deps:** T0.1
**Files:** `Cargo.toml` (workspace), `rust-toolchain.toml`, `clippy.toml`, `rustfmt.toml`
**Deliverable:** Empty workspace declaring all 15 crates from spec §11 as members. `rust-toolchain.toml` pins Rust 1.82+. `clippy.toml` denies `unwrap_used` and `expect_used` outside tests.
**Tests:** `crates/_workspace_smoke/tests/smoke.rs` — single test asserting workspace builds.
**Verify:**

```bash
cd /Users/ryanoboyle/pellucid && cargo check --workspace
cd /Users/ryanoboyle/pellucid && cargo fmt --check
```

### T0.3 — Crate skeletons (15 crates) [setup][rust]
**Deps:** T0.2
**Files:** `crates/{pellucid-core,pellucid-db,pellucid-cache,pellucid-streams,pellucid-seeders,pellucid-gateway,pellucid-handlers,pellucid-auth,pellucid-ml,pellucid-correlation,pellucid-workers,pellucid-codegen,pellucid-tauri,pellucid-sidecar-bin,pellucid-edge-bin,pellucid-relay-bin}/Cargo.toml` + `src/lib.rs` (or `main.rs`)
**Deliverable:** Each crate compiles empty. Library crates expose `pub fn version() -> &'static str { env!("CARGO_PKG_VERSION") }` so we can write the universal smoke test.
**Tests:** Per-crate unit test `#[test] fn version_is_set() { assert!(!version().is_empty()); }` — yes, even this trivial method gets its test, per the universal mandate.
**Verify:**

```bash
cargo nextest run --workspace
```

All 15 trivial tests pass.

### T0.4 — Bun + webview scaffold [setup][webview]
**Deps:** T0.1
**Files:** `package.json` (root + `webview/`), `bun.lockb`, `webview/vite.config.ts`, `webview/tsconfig.json`, `webview/index.html`, `webview/src/main.tsx`, `webview/src/App.tsx`
**Deliverable:** Vite 6 + React 19 + TypeScript scaffold. `bun install` succeeds. `bun run dev` opens a window saying "Pellucid".
**Tests:**
- Unit: `webview/src/App.test.tsx` — renders without crashing.
- Integration: `webview/integration/boot.test.tsx` — `<App>` mounts and reaches a stable state in jsdom.
- E2E: `e2e/smoke.spec.ts` — Playwright opens dev server and asserts "Pellucid" text appears.

**Verify:**

```bash
bun install
bun run --filter=webview build
bun test
bunx playwright test e2e/smoke.spec.ts
```

### T0.5 — Rust test tooling install [setup][rust]
**Deps:** T0.3
**Files:** `Cargo.toml` workspace `[workspace.dependencies]` adds `wiremock`, `proptest`, `vcr-rs`. `.cargo/config.toml` configures nextest profile. `tools/install-rust-tools.sh` installs `cargo-nextest`, `cargo-llvm-cov`, `cargo-audit`, `cargo-deny`.
**Deliverable:** All tools available in CI and locally.
**Tests:** `crates/pellucid-core/tests/tooling.rs` — single test that proptest runs and wiremock binds a port.
**Verify:**

```bash
cargo nextest --version
cargo llvm-cov --version
cargo audit --version
cargo nextest run -p pellucid-core --test tooling
```

### T0.6 — Tauri 2 host scaffold [setup][rust]
**Deps:** T0.3
**Files:** `crates/pellucid-tauri/Cargo.toml`, `crates/pellucid-tauri/tauri.conf.json` (+ `tauri.tech.conf.json`, `tauri.finance.conf.json`, `tauri.commodity.conf.json`, `tauri.happy.conf.json`), `crates/pellucid-tauri/src/main.rs` (window only, no IPC yet), `crates/pellucid-tauri/icons/` from a placeholder icon set.
**Deliverable:** `cargo tauri dev` opens a real native window pointing at `bun run --filter=webview dev` localhost.
**Tests:**
- Unit: `crates/pellucid-tauri/src/main.rs` `#[cfg(test)] mod tests` for any helper.
- Integration: `crates/pellucid-tauri/tests/window.rs` — boots Tauri in headless mode, asserts window created.
- E2E: `e2e/desktop/window.spec.ts` — Playwright + tauri-driver opens app, asserts title.

**Verify:**

```bash
cargo nextest run -p pellucid-tauri
bunx playwright test --project=desktop e2e/desktop/window.spec.ts
```

### T0.7 — TS test infrastructure [setup][webview]
**Deps:** T0.4
**Files:** `webview/test/setup.ts`, `webview/test/utils.tsx` (testing-library wrappers), `webview/.bunfig.toml`, dependencies for `@testing-library/react`, `polly.js`, `convex-test`.
**Deliverable:** `bun test` discovers and runs both `*.test.ts` and `*.test.tsx` with React + jsdom + Convex test env wired.
**Tests:** Self-test — `webview/test/setup.test.ts` asserts the test environment exposes `document`, `localStorage`, `fetch`.
**Verify:**

```bash
bun test webview/test
```

### T0.8 — Playwright + tauri-driver [setup][webview]
**Deps:** T0.4, T0.6
**Files:** `playwright.config.ts`, `e2e/fixtures/`, `e2e/global-setup.ts`, `e2e/desktop/setup.ts` (tauri-driver bridge).
**Deliverable:** Two Playwright projects — `web` (against `bun run dev`) and `desktop` (against `cargo tauri dev` via tauri-driver). Visual regression baseline directory created.
**Tests:** `e2e/health.spec.ts` (web) + `e2e/desktop/health.spec.ts` (desktop) — each loads the page and asserts no console errors.
**Verify:**

```bash
bunx playwright test
```

### T0.9 — Convex project init [setup][convex]
**Deps:** T0.1
**Files:** `convex/schema.ts`, `convex/_generated/` (gitignored), `convex/auth.config.ts`, `convex.json`
**Deliverable:** `bunx convex dev` registers an empty deployment. Schema declares `entitlements`, `webhook_seen`, `contact_submissions`, `waitlist` tables (per spec §15).
**Tests:**
- Unit: `convex/schema.test.ts` — schema parses; required indexes present.
- Integration: `convex/tests/empty.test.ts` — convex-test boots an in-memory deployment, no errors.
- E2E: handled at M1 webhook task.

**Verify:**

```bash
bunx convex codegen
bun test convex
```

### T0.10 — Justfile + tooling scripts [setup]
**Deps:** T0.2, T0.4
**Files:** `Justfile`, `tools/check-cache-keys.ts`, `tools/check-csp.ts`, `tools/check-edge-imports.ts`, `tools/build-csp.ts`, `tools/version-sync.ts`. Initial implementations are real (no stubs): `check-cache-keys.ts` parses Rust handler files via tree-sitter and compares cache-key strings against handler request fields.
**Deliverable:** `just check` runs the full universal CI gate (§1.3). `just dev-desktop`, `just dev-edge`, `just dev-relay`, `just dev-web` all work.
**Tests:**
- Unit: each `tools/*.ts` file has `tools/*.test.ts` exercising every code path.
- Integration: `tools/check-cache-keys.test.ts` runs against a fixture handler file with both correct and incorrect cache keys; asserts both detection paths.
- E2E: not applicable (build tool).

**Verify:**

```bash
bun test tools
just check   # passes on empty workspace
```

### T0.11 — CI workflows [setup]
**Deps:** T0.10
**Files:** `.github/workflows/{typecheck.yml,lint.yml,proto-check.yml,build-desktop.yml,docker-publish.yml,test-linux-app.yml,audit.yml,coverage.yml}`
**Deliverable:** Each workflow runs the corresponding `just` task. PRs from forks blocked from secrets. Coverage report uploaded as artifact.
**Tests:** `.github/workflows/test-workflows.yml` self-tests workflow YAML via `actionlint`.
**Verify:**

```bash
bunx --bun actionlint
just check   # local equivalent
```

### T0.12 — Pre-push hook [setup]
**Deps:** T0.10, T0.11
**Files:** `.husky/pre-push`, `.husky/_/husky.sh`, `package.json` adds `prepare: husky`.
**Deliverable:** `git push` runs `just check` and refuses on failure (parity with spec OP-19).
**Tests:** `tools/test-prepush.sh` simulates a push with a known-bad change and asserts non-zero exit.
**Verify:**

```bash
bash tools/test-prepush.sh
```

### T0 Gate
**Verify:**

```bash
cd /Users/ryanoboyle/pellucid && just check
```

All commands in §1.3 pass. Repo is bootstrapped, polyglot, CI-gated, test-driven from day one.

---

## Milestone 0 — Foundation (Spec §28 M0; week 1–2)

**Goal:** `pellucid-core`, `pellucid-db`, `pellucid-cache`, Tauri sidecar with rotated token (H1 fix), Bun+Vite+React webview that completes one round-trip through the sidecar's echo handler.

### T1.1 — `pellucid-core` types [rust]
**Deps:** T0
**Files:** `crates/pellucid-core/src/{lib.rs,envelope.rs,seed_meta.rs,cache_tier.rs,fnv.rs,error.rs,time.rs,id.rs}`
**Deliverable:** `Envelope<T>`, `SeedEnvelope`, `SeedMeta`, `CacheTier` enum (FAST/MEDIUM/SLOW/SLOW_BROWSER/STATIC/DAILY/NO_STORE), `FnvHasher`, `now_ms()`, `RunId(Uuid)`, error enum. Pure types, no I/O.
**Tests:**
- Unit: every type has its own `*.test.rs` (or inline `mod tests`) exercising serialization round-trip, Default, equality, error propagation. Property test (`proptest`) on `FnvHasher` confirms it never panics on arbitrary bytes and matches a reference implementation.
- Integration: `crates/pellucid-core/tests/envelope_compat.rs` — round-trips an envelope through serde_json against a fixture from the original WorldMonitor (`tests/fixtures/envelope.json`).
- E2E: not applicable (no I/O).

**Verify:**

```bash
cargo nextest run -p pellucid-core
cargo llvm-cov -p pellucid-core --fail-under-lines 90
```

### T1.2 — `pellucid-db` migrations + pool [rust]
**Deps:** T1.1
**Files:** `crates/pellucid-db/src/{lib.rs,pool.rs,migrate.rs}`, `crates/pellucid-db/migrations/0001_initial.sql` (every table from spec §6.1 + §6.2).
**Deliverable:** `open(path) -> SqlitePool` enforces all PRAGMAs from spec §6. `migrate(&pool)` applies all migrations idempotently. FTS5, R*Tree, sqlite-vec extensions loaded.
**Tests:**
- Unit: `pool.rs` `#[cfg(test)]` — open against `:memory:`, assert PRAGMA values match spec §6 (WAL, synchronous=NORMAL, busy_timeout=5000, mmap_size=268435456, foreign_keys=ON, cache_size=-65536).
- Integration: `crates/pellucid-db/tests/migrate.rs` — opens fresh DB, applies migrations twice (idempotency), inserts into every table, asserts FTS5 + R*Tree + sqlite-vec virtual tables work.
- E2E: not applicable.

**Verify:**

```bash
cargo nextest run -p pellucid-db
```

### T1.3 — `pellucid-cache` KV with stampede coalescing [rust]
**Deps:** T1.2
**Files:** `crates/pellucid-cache/src/{lib.rs,kv.rs,coalesce.rs,negative.rs,batch.rs}`
**Deliverable:** `cached_fetch_json<T, F>(pool, key, tier, fetcher)` with stampede coalescing (spec §7.2), negative-cache sentinel (spec §7.3), batch reads (`get_cached_json_batch`). Direct port of `server/_shared/redis.ts` semantics to SQLite.
**Tests:**
- Unit: `coalesce.rs` `#[cfg(test)]` — N concurrent calls with the same key result in exactly 1 fetcher invocation. `negative.rs` `#[cfg(test)]` — null result triggers sentinel, sentinel returns Ok(None) without invoking fetcher.
- Integration: `crates/pellucid-cache/tests/stampede.rs` — 100 parallel `cached_fetch_json` calls on cold cache; assertion: 1 upstream call, 100 successful returns. `crates/pellucid-cache/tests/batch.rs` — `get_cached_json_batch` against pre-populated SQLite.
- E2E: not applicable (used by handlers; handler-level e2e in M1).

**Verify:**

```bash
cargo nextest run -p pellucid-cache
```

### T1.4 — `pellucid-cache` rate limiting [rust]
**Deps:** T1.3
**Files:** `crates/pellucid-cache/src/rate_limit.rs`, migration `0002_rate_limit_indexes.sql`
**Deliverable:** Sliding-window rate limit over SQLite. Three buckets: `rl:ip:<ip>`, `rl:ep:<rpc>`, `rl:agg:<ip>` (umbrella cap — **M8 fix**).
**Tests:**
- Unit: `rate_limit.rs` `#[cfg(test)]` — exhausting an endpoint bucket trips at exact cap; clock advance frees the window.
- Integration: `crates/pellucid-cache/tests/rate_limit.rs` — concurrent requests from same IP across two endpoints; aggregate cap correctly trips before either specific cap.
- E2E: at gateway level in M1.

**Verify:**

```bash
cargo nextest run -p pellucid-cache --test rate_limit
```

### T1.5 — Zustand stores scaffold [webview]
**Deps:** T0.4, T0.7
**Files:** `webview/src/state/{useAuthStore.ts,useVariantStore.ts,useUiStore.ts,usePanelStore.ts,useDataStore.ts,useMapStore.ts,useNewsStore.ts,useCorrelationStore.ts,useBootStore.ts}`
**Deliverable:** All 9 stores from spec §13.1 created with typed initial state, `subscribeWithSelector` middleware, and persistence config where required (Tauri store on desktop, IndexedDB/`localStorage` on web).
**Tests:**
- Unit: per-store `*.test.ts` — every action, selector, and reaction has a test.
- Integration: `webview/integration/state-cross-store.test.ts` — variant change → reset map layers (port of `App.ts:424-449`); assertion is a real reaction firing in jsdom.
- E2E: deferred to M3 (where panels actually consume the state).

**Verify:**

```bash
bun test webview/src/state
bun test webview/integration/state-cross-store.test.ts
```

### T1.6 — Radix + Tailwind base [webview]
**Deps:** T0.4
**Files:** `webview/tailwind.config.ts`, `webview/src/styles/globals.css`, `webview/src/styles/variants/{base,tech,finance,commodity,happy}.css`, `webview/src/components/primitives/{Button.tsx,Dialog.tsx,Tooltip.tsx,DropdownMenu.tsx,Tabs.tsx,Toast.tsx,Slider.tsx,Toolbar.tsx,Toggle.tsx,Collapsible.tsx,Popover.tsx,ScrollArea.tsx}`
**Deliverable:** All Radix primitives wrapped with Pellucid styles. Variant CSS swap via `data-variant` on `<html>`.
**Tests:**
- Unit: each primitive `<Button>`, `<Dialog>`, etc. has a `*.test.tsx` covering all variants and states.
- Integration: `webview/integration/variant-switch.test.tsx` — switching `useVariantStore` value updates `data-variant` and the rendered theme tokens change.
- E2E: `e2e/visual/variants.spec.ts` — for each of 5 variants, render the primitive showcase page and capture golden screenshot.

**Verify:**

```bash
bun test webview/src/components/primitives
bunx playwright test e2e/visual/variants.spec.ts
```

### T1.7 — `pellucid-tauri` IPC + sidecar spawn [rust]
**Deps:** T0.6, T1.2
**Files:** `crates/pellucid-tauri/src/{ipc.rs,sidecar.rs,vault.rs}`
**Deliverable:** IPC commands: `get_local_api_port`, `get_local_api_token`, `refresh_secrets`, `get_variant`, `set_variant`, `request_updater_check`, `open_external` (per spec §10.2). Vault implements consolidated keychain entry (`pellucid:secrets-vault:v1`) with platform listener for keychain changes (**M9 fix**).
**Tests:**
- Unit: `ipc.rs` `#[cfg(test)]` — every command has unit test against a mock state.
- Integration: `crates/pellucid-tauri/tests/ipc_real_state.rs` — Tauri test harness, real `LocalApiState`, every command invoked end-to-end.
- E2E: `e2e/desktop/ipc.spec.ts` — Tauri-driver invokes `__TAURI__.invoke('get_local_api_port')` from webview and asserts a numeric port returned.

**Verify:**

```bash
cargo nextest run -p pellucid-tauri
bunx playwright test --project=desktop e2e/desktop/ipc.spec.ts
```

### T1.8 — H1 FIX — Token rotation [rust][fix]
**Deps:** T1.7
**Files:** `crates/pellucid-tauri/src/token_rotation.rs`, modifies `crates/pellucid-tauri/src/ipc.rs` to consult rotation state.
**Deliverable:** Background `tokio::spawn` rotates the token every 5 minutes. Sidecar accepts current OR previous token during 30 s overlap. Webview receives `token_rotated` event.
**Tests:**
- Unit: `token_rotation.rs` `#[cfg(test)]` — manual clock-tick advances rotation; current/previous accepted; older rejected.
- Integration: `crates/pellucid-tauri/tests/regression_h1.rs` — **regression test**. Boots Tauri + sidecar, captures token, advances 5 min via mock clock, captures new token, asserts they differ; confirms sidecar accepts both during overlap window.
- E2E: `e2e/desktop/token_rotation.spec.ts` — runs for 35 minutes (CI-only, slow tag), captures ≥ 6 distinct tokens via IPC sniffer, asserts rotation cadence.

**Verify:**

```bash
cargo nextest run -p pellucid-tauri --test regression_h1
# Confirm fix-fail: temporarily revert rotation logic, run test, expect failure.
bunx playwright test --project=desktop e2e/desktop/token_rotation.spec.ts --grep="@slow"
```

**Evidence required (CLAUDE.md):** show output where reverting fixes the test fails, and re-applying makes it pass.

### T1.9 — `pellucid-sidecar-bin` echo handler [rust]
**Deps:** T1.7
**Files:** `crates/pellucid-sidecar-bin/src/main.rs`, listens on `127.0.0.1:0`, prints `PORT=<n>`, mounts a single `/api/echo` Axum route that requires the bearer token.
**Deliverable:** Sidecar runs, accepts authenticated requests, returns echo payload.
**Tests:**
- Unit: `main.rs` `#[cfg(test)]` for any helpers.
- Integration: `crates/pellucid-sidecar-bin/tests/echo.rs` — boots sidecar on random port, captures port from stdout, sends request with valid token (200) and invalid (401).
- E2E: `e2e/desktop/echo.spec.ts` — webview calls `fetch(toApiUrl('/api/echo'))` and asserts round-trip.

**Verify:**

```bash
cargo nextest run -p pellucid-sidecar-bin
bunx playwright test --project=desktop e2e/desktop/echo.spec.ts
```

### T1.10 — Webview runtime helpers [webview]
**Deps:** T1.7, T1.9
**Files:** `webview/src/services/runtime.ts` ports of `resolveLocalApiPort`, `getLocalApiPort`, `detectDesktopRuntime`, `isDesktopRuntime`, `getApiBaseUrl`, `toApiUrl`, `installRuntimeFetchPatch`, `installWebApiRedirect`, `class VisibilityHub`, `startSmartPollLoop` (spec §3.3 mapping of `src/services/runtime.ts:30-749`).
**Deliverable:** Webview can build URLs against either Tauri sidecar or `api.worldmonitor.app` based on `import.meta.env.VITE_TARGET`.
**Tests:**
- Unit: every function has a `*.test.ts`. `installRuntimeFetchPatch` is tested by replacing `globalThis.fetch` and asserting the patch redirects desktop URLs.
- Integration: `webview/integration/runtime.test.ts` — mocked `__TAURI__` global, asserts URL builds for desktop; cleared global, asserts web URL.
- E2E: `e2e/runtime/url-build.spec.ts` (web) + `e2e/desktop/url-build.spec.ts` (desktop) — both call `toApiUrl('/api/echo')` and assert returned URL points at the right host.

**Verify:**

```bash
bun test webview/src/services/runtime
bunx playwright test e2e/runtime e2e/desktop/url-build.spec.ts
```

### T1.11 — `<App>` 8-phase boot scaffold [webview]
**Deps:** T1.5, T1.6, T1.10
**Files:** `webview/src/app/boot.ts`, `webview/src/state/useBootStore.ts` (extends T1.5), `webview/src/App.tsx` updates.
**Deliverable:** Boot state machine reaches the placeholder for each of P1–P8 (spec §2 OP-2). Phases are visible in DevTools via Zustand devtools; error in any phase short-circuits cleanly.
**Tests:**
- Unit: `boot.ts` `*.test.ts` — every phase transition (start → P1 → P2 → ... → P8) has a test that asserts state shape.
- Integration: `webview/integration/boot-flow.test.ts` — full boot in jsdom against fakes for storage, IPC, fetch; asserts terminal state == `ready`.
- E2E: `e2e/boot.spec.ts` (web + desktop) — Playwright observes DOM transition through every phase indicator.

**Verify:**

```bash
bun test webview/src/app/boot
bunx playwright test e2e/boot.spec.ts
```

### M0 Gate [gate]
**Verify:**

```bash
just check
cargo llvm-cov --workspace --fail-under-lines 90
bunx playwright test
```

Exit criteria (spec §28 M0): `cargo nextest run --workspace` green; `bun test` green; Tauri dev launches and webview makes one round-trip through sidecar (echo handler). **Plus universal mandate:** every public method added in M0 has unit + integration + e2e coverage.

---

## Milestone 1 — Gateway + first vertical (Spec §28 M1; week 3–5)

**Goal:** 14-stage gateway, Clerk + entitlement (with H2 fix), one canonical handler (`aviation/v1/get-flight-status`), bootstrap shell, variant detection. Demonstrates the entire request path through the system.

### T2.1 — `pellucid-gateway` Tower middleware skeleton [rust]
**Deps:** T1.3, T1.4
**Files:** `crates/pellucid-gateway/src/{lib.rs,stages/mod.rs,stages/origin.rs,stages/cors.rs,stages/preflight.rs,stages/tier_gate.rs,stages/clerk_session.rs,stages/api_key.rs,stages/entitlement.rs,stages/endpoint_rate.rs,stages/global_rate.rs,stages/handler_boundary.rs,stages/header_merge.rs,stages/etag.rs,stages/cache_control.rs}`, `crates/pellucid-gateway/src/router.rs`, `crates/pellucid-gateway/src/error_mapper.rs`
**Deliverable:** All 14 stages from spec §8.3. `build_router(handlers)` returns an Axum router with the full pipeline.
**Tests:**
- Unit: every stage `mod tests` — failure paths (403, 401, 403, 429, 429, 404, 405, 500, 304) have explicit tests; happy path has explicit tests.
- Integration: `crates/pellucid-gateway/tests/pipeline.rs` — single in-process Axum server with the full pipeline; 50 cases covering: bad origin, missing CORS, OPTIONS preflight, missing Clerk on tier-gated, invalid API key, denied entitlement, exceeded endpoint rate, exceeded global rate, route hit, route miss, ETag 304, custom cache header.
- E2E: deferred to T2.5 (where the first real handler exists).

**Verify:**

```bash
cargo nextest run -p pellucid-gateway
```

### T2.2 — `pellucid-auth` Clerk JWT verification [rust]
**Deps:** T2.1
**Files:** `crates/pellucid-auth/src/{lib.rs,clerk.rs,jwks.rs}`
**Deliverable:** `verify_jwt(token) -> Result<Claims>` against Clerk JWKS, cached 5 min via `tokio::sync::RwLock<Option<JwksCache>>`. JWKS URL configurable.
**Tests:**
- Unit: `clerk.rs` `#[cfg(test)]` — valid token, expired, wrong issuer, wrong audience, malformed, missing claim — each gets a test using a fixture key pair.
- Integration: `crates/pellucid-auth/tests/clerk_e2e.rs` — `wiremock` server hosts a JWKS document; verify_jwt round-trip against signed and unsigned tokens.
- E2E: at gateway level T2.5.

**Verify:**

```bash
cargo nextest run -p pellucid-auth --test clerk_e2e
```

### T2.3 — `pellucid-auth` entitlement check + H2 FIX [rust][fix]
**Deps:** T2.2, T1.3
**Files:** `crates/pellucid-auth/src/{entitlement.rs,endpoint_tiers.rs}`
**Deliverable:** `Decision { Allow, Deny, UpstreamDown }` three-arm. SQLite cache (15 min TTL) → Convex `internal-entitlements` HTTP fallback. `UpstreamDown` returned when both layers fail.
**Tests:**
- Unit: `entitlement.rs` `#[cfg(test)]` — cache hit (Allow), cache hit but expired (Deny when tier insufficient), cache miss + Convex success, cache miss + Convex 5xx (UpstreamDown), cache miss + Convex unreachable (UpstreamDown).
- Integration: `crates/pellucid-auth/tests/regression_h2.rs` — **regression test**. Boots gateway with entitlement stage; one request hits with cache cold + Convex returning 5xx; assert response = 503 + `Retry-After: 30`. Temporary revert of three-arm Decision causes test to fail.
- E2E: `e2e/auth/upstream-down.spec.ts` — webview calls a tier-2 endpoint with Convex stubbed offline (via wiremock proxy); UI shows outage banner not upgrade prompt.

**Verify:**

```bash
cargo nextest run -p pellucid-auth --test regression_h2
bunx playwright test e2e/auth/upstream-down.spec.ts
```

**Evidence:** revert + re-apply demonstration.

### T2.4 — `pellucid-auth` HMAC identity signing (OP-12) [rust]
**Deps:** T2.2
**Files:** `crates/pellucid-auth/src/{hmac.rs,identity.rs}`
**Deliverable:** `sign_user_id_hmac(user_id, secret) -> base64` + `verify_user_id_hmac(value, sig, secret) -> bool` (constant-time via `subtle`). Port of `convex/lib/identitySigning.ts:29-72`.
**Tests:**
- Unit: every function tested with valid/invalid sigs, empty input, oversized input, time-attack property test (proptest).
- Integration: `crates/pellucid-auth/tests/identity_compat.rs` — given user_id and secret, asserts signature byte-equal to a fixture produced by the original Node implementation.
- E2E: at M5 webhook task.

**Verify:**

```bash
cargo nextest run -p pellucid-auth --test identity_compat
```

### T2.5 — `pellucid-handlers` aviation/v1/get-flight-status [rust]
**Deps:** T2.1, T1.3, T2.3
**Files:** `crates/pellucid-handlers/src/{lib.rs,aviation/mod.rs,aviation/v1/get_flight_status.rs}`, `crates/pellucid-codegen/src/build.rs` registered for sebuf compilation.
**Deliverable:** First real RPC handler. Cache key `aviation:status:{flight}:{date}:{origin}:v1`. Tier `FAST`. Calls `pellucid-streams::aviationstack::fetch_flight` (skeleton, real call deferred to T3.x; here we use `wiremock` for the upstream during integration tests, but the production binary calls the real API).
**Tests:**
- Unit: `get_flight_status.rs` `mod tests` — cache key assembly, request validation, error mapping.
- Integration: `crates/pellucid-handlers/tests/aviation.rs` — full pipeline: gateway + handler + cache + wiremock'd aviationstack. Cold cache invokes upstream once, subsequent calls hit cache.
- E2E: `e2e/aviation/flight-status.spec.ts` — webview calls `/api/aviation/v1/get-flight-status?flight=AA100&date=2026-04-25&origin=JFK`; against `pellucid-edge-bin` running in test mode with a wiremock'd upstream.

**Verify:**

```bash
cargo nextest run -p pellucid-handlers --test aviation
bunx playwright test e2e/aviation/flight-status.spec.ts
```

### T2.6 — `pellucid-edge-bin` minimum viable [rust]
**Deps:** T2.5
**Files:** `crates/pellucid-edge-bin/src/{main.rs,config.rs,middleware.rs}`
**Deliverable:** Binary that opens SQLite, mounts `pellucid-gateway::build_router(handlers)`, listens on `0.0.0.0:8080`, serves the aviation handler.
**Tests:**
- Unit: `config.rs` `#[cfg(test)]` — env parsing exhaustive.
- Integration: `crates/pellucid-edge-bin/tests/boot.rs` — boots binary, hits `/api/aviation/v1/get-flight-status`, asserts 200 + envelope shape.
- E2E: `e2e/edge/aviation.spec.ts` — Playwright against the running binary.

**Verify:**

```bash
cargo nextest run -p pellucid-edge-bin --test boot
bunx playwright test e2e/edge/aviation.spec.ts
```

### T2.7 — Bootstrap two-tier hydration shell [webview][rust]
**Deps:** T2.6, T1.11
**Files:** Rust: `crates/pellucid-handlers/src/bootstrap/v1/get.rs` with `tier=fast|slow|both` query and `keys=` overrides. Webview: `webview/src/services/bootstrap.ts` ports `getHydratedData`, `markBootstrapAsLive`, `getBootstrapHydrationState`, `fetchBootstrapData` (spec §3.3 mapping).
**Deliverable:** OP-4 satisfied: 67 fast keys + 45 slow keys + 112 total (constants in `crates/pellucid-handlers/src/bootstrap/keys.rs` matching original `BOOTSTRAP_CACHE_KEYS`). Two-tier concurrent fetch from webview.
**Tests:**
- Unit: Rust handler — partial-miss path returns 200 with `missing[]`; all-miss path returns 503 + `Retry-After` (**M4 fix**). Webview — every helper has a test.
- Integration: `crates/pellucid-handlers/tests/bootstrap.rs` — populate 50 keys, request all 112 (fast tier), assert 200 + 50 hits + 17 in `missing[]`. Then clear cache, request again, assert 503 + `Retry-After`.
- E2E: `e2e/boot/bootstrap.spec.ts` — webview boots, fast tier returns ≤ 3 s, slow tier returns ≤ 5 s, panels see hydrated data.

**Verify:**

```bash
cargo nextest run -p pellucid-handlers --test bootstrap
bun test webview/src/services/bootstrap
bunx playwright test e2e/boot/bootstrap.spec.ts
```

### T2.8 — Variant detection chain [webview]
**Deps:** T1.5, T1.6
**Files:** `webview/src/config/variant.ts`, `webview/src/config/variants/{base,tech,finance,commodity,happy}.ts`, `webview/src/state/reactions.ts` (cross-store reactions for variant change).
**Deliverable:** OP-13 satisfied. Variant change resets `useMapStore.layers`, disables panels not in target variant's allow-list, seeds defaults, records migration keys.
**Tests:**
- Unit: every function in `variant.ts` and `reactions.ts` tested with all 5 variants.
- Integration: `webview/integration/variant-switch.test.tsx` — mount full app, switch variant, assert reactions fire.
- E2E: `e2e/visual/variants.spec.ts` (extends T1.6) — switch variant in UI, verify map layers reset and disabled panels become hidden.

**Verify:**

```bash
bun test webview/src/config webview/src/state/reactions.test.ts
bunx playwright test e2e/visual/variants.spec.ts
```

### T2.9 — H4 FIX — Single tier-based gating [rust][fix]
**Deps:** T2.3
**Files:** `crates/pellucid-auth/src/endpoint_tiers.rs` becomes the strict superset (per spec §14.4); `tools/migrate-premium-paths.ts` reads original `PREMIUM_RPC_PATHS` reference list and emits the missing 33 entries with their tier.
**Deliverable:** Legacy `PREMIUM_RPC_PATHS` Bearer-role path absent from gateway. All 37 endpoints (4 original + 33 migrated) gated by `ENDPOINT_ENTITLEMENTS` map.
**Tests:**
- Unit: `endpoint_tiers.rs` `mod tests` — exhaustive map size + every entry's tier.
- Integration: `crates/pellucid-gateway/tests/regression_h4.rs` — **regression test**. Each of 37 endpoints called: with insufficient tier → 403; with sufficient → 200 (handler stub). Reverting to legacy gating triggers 401 paths instead — test fails.
- E2E: `e2e/auth/premium-gating.spec.ts` — sample 5 of the 37 endpoints, log in as free-tier and pro-tier, assert respective UX.

**Verify:**

```bash
cargo nextest run -p pellucid-gateway --test regression_h4
bunx playwright test e2e/auth/premium-gating.spec.ts
```

### T2.10 — `tools/check-cache-keys.ts` real implementation + M1 FIX [setup][fix]
**Deps:** T0.10
**Files:** `tools/check-cache-keys.ts` (full implementation, not the smoke version from T0.10).
**Deliverable:** Tool parses every `cached_fetch_json(...)` invocation in `crates/pellucid-handlers/**/*.rs`, extracts the key string and the request-body fields referenced in the same handler body. Fails if any non-hardcoded handler references a request field absent from the key.
**Tests:**
- Unit: each function in the parser tested.
- Integration: `tools/check-cache-keys.test.ts` — fixture handler files: one correct, one with missing field, one hardcoded; asserts 0 errors / 1 error / 0 errors respectively.
- E2E: not applicable.

**Verify:**

```bash
bun test tools/check-cache-keys.test.ts
just check   # fails if any current handler has key drift; currently aviation handler is the only one — must pass
```

### T2.11 — Aviation panel skeleton [webview][panel]
**Deps:** T2.5, T2.7, T1.11
**Files:** `webview/src/panels/aviation/AviationPanel.tsx` (placeholder body — full panel is one of M3's), `webview/src/data/loaders/aviation.ts`, panel registered in `usePanelStore`.
**Deliverable:** Single panel renders on the grid showing flight status data fetched via `/api/aviation/v1/get-flight-status`. End-to-end demonstration of the full path.
**Tests:**
- Unit: `AviationPanel.test.tsx` (states: loading, error, ready, locked).
- Integration: `webview/integration/aviation-panel.test.tsx` — mount `<AviationPanel>` with mocked store; assertion: data loader was called with correct params.
- E2E: `e2e/panels/aviation.spec.ts` (web + desktop) — full stack, real handler, real upstream sandbox: panel shows real flight data.

**Verify:**

```bash
bun test webview/src/panels/aviation
bunx playwright test e2e/panels/aviation.spec.ts
```

### M1 Gate [gate]
Spec §28 M1 exit: aviation panel renders flight status from desktop and from `worldmonitor.app`; entitlement 503 path verified; H1, H2, H4 regression tests green.

```bash
just check
cargo nextest run --workspace
bunx playwright test
```

---

## Milestone 2 — Streams + seeders (Spec §28 M2; week 6–9)

**Goal:** AIS, OpenSky, RSS, OREF stream clients in Rust; ~30 seeders; relay binary deployed to Fly.io with C1 fix; bootstrap returns hydrated data for 30 cache keys.

### T3.1 — `pellucid-streams` AIS client [rust]
**Deps:** T1.2
**Files:** `crates/pellucid-streams/src/{lib.rs,ais.rs,types.rs}`
**Deliverable:** Tokio-tungstenite client to `wss://stream.aisstream.io/v0/stream`. Exponential-backoff reconnect. HIGH/LOW watermark queue. Decoded messages → broadcast channel.
**Tests:**
- Unit: `ais.rs` `mod tests` — backoff math, watermark logic, decode of known message types.
- Integration: `crates/pellucid-streams/tests/ais.rs` — wiremock-WS server pushes a known message stream; consumer asserts decoded items match.
- E2E: `e2e/relay/ais.spec.ts` (CI-tagged `@slow`, against the staging relay) — 5-minute capture asserts ≥ 1 message received.

**Verify:**

```bash
cargo nextest run -p pellucid-streams --test ais
```

### T3.2 — `pellucid-streams` OpenSky client [rust]
**Deps:** T3.1
**Files:** `crates/pellucid-streams/src/opensky.rs`
**Deliverable:** OAuth2 client_credentials flow with `oauth2` crate; token cached with 60 s buffer; mutex serializes refresh; LRU positive cache (1024, 60 s — **M7 fix**); negative sentinel (30 s); 90 s 429 cooldown.
**Tests:**
- Unit: every function tested; mutex contention property test.
- Integration: `crates/pellucid-streams/tests/opensky.rs` — wiremock OpenSky API; assert single token refresh under 100 concurrent requests.
- E2E: at relay level T3.10.

**Verify:**

```bash
cargo nextest run -p pellucid-streams --test opensky
```

### T3.3 — `pellucid-streams` RSS client [rust]
**Deps:** T1.2
**Files:** `crates/pellucid-streams/src/{rss/mod.rs,rss/allowed_domains.rs}`
**Deliverable:** `feed-rs` parser. Allowlist from `crates/pellucid-streams/src/rss/allowed_domains.rs` (port of `shared/rss-allowed-domains.cjs`). 5 min positive / 1 min negative cache. In-flight dedup via `dashmap`.
**Tests:**
- Unit: parser handles RSS 1.0/2.0, Atom, malformed XML; allowlist enforces deny.
- Integration: `crates/pellucid-streams/tests/rss.rs` — wiremock feeds; assert dedup, caching.
- E2E: at relay level.

**Verify:**

```bash
cargo nextest run -p pellucid-streams --test rss
```

### T3.4 — `pellucid-streams` OREF client [rust]
**Deps:** T1.2
**Files:** `crates/pellucid-streams/src/oref.rs`, `crates/pellucid-streams/src/ja3.rs`
**Deliverable:** Reqwest client with custom `rustls` ClientConfig spoofing Chrome JA3. Residential proxy fallback. History persisted to `kv_envelope` (`relay:oref:history:v1`).
**Tests:**
- Unit: JA3 fingerprint computation matches a known Chrome value; proxy fallback triggers on direct failure.
- Integration: `crates/pellucid-streams/tests/oref.rs` — wiremock with Server header check; asserts request goes through with expected fingerprint.
- E2E: at relay level.

**Verify:**

```bash
cargo nextest run -p pellucid-streams --test oref
```

### T3.5 — `pellucid-seeders` atomic_publish [rust]
**Deps:** T1.3
**Files:** `crates/pellucid-seeders/src/{lib.rs,atomic_publish.rs,scheduler.rs,validate.rs,envelope.rs,locks.rs}`
**Deliverable:** Port of `_seed-utils.mjs:170-210` per spec §7.4. Lock via `BEGIN IMMEDIATE` over `seed_lock` table. Validate envelope shape + 5 MB cap. Staging key → canonical → seed_meta → release lock.
**Tests:**
- Unit: each step (acquire_lock, validate, envelope_build, ttl_compute, release_lock) tested in isolation.
- Integration: `crates/pellucid-seeders/tests/atomic_publish.rs` — concurrent publish attempts on the same key; only one wins, the other waits or fails clean.
- E2E: at relay binary level.

**Verify:**

```bash
cargo nextest run -p pellucid-seeders --test atomic_publish
```

### T3.6 — `pellucid-seeders` scheduler [rust]
**Deps:** T3.5
**Files:** `crates/pellucid-seeders/src/scheduler.rs`, `crates/pellucid-seeders/src/registry.rs`
**Deliverable:** Static `phf::Map<&'static str, Cadence>` from spec §17.7 — registers cadences, dispatches workers via `tokio::time::interval`. Counter via `metrics::counter!` for skip events (**M3 fix**).
**Tests:**
- Unit: `Cadence` parsing; scheduler dispatch order under simulated time.
- Integration: `crates/pellucid-seeders/tests/scheduler.rs` — registers 3 fake seeders with different cadences; mock clock advance; asserts each fires at expected times.
- E2E: at relay level.

**Verify:**

```bash
cargo nextest run -p pellucid-seeders --test scheduler
```

### T3.7 — H3 FIX — Theater-posture seeder direct-call [rust][fix]
**Deps:** T3.2, T3.5, T3.6
**Files:** `crates/pellucid-seeders/src/theater_posture/mod.rs`
**Deliverable:** Calls `pellucid-streams::opensky::fetch_box(bbox)` **directly in-process**. No HTTP loopback to own server (per spec §17.7 H3 fix).
**Tests:**
- Unit: bbox computation; result transformation.
- Integration: `crates/pellucid-seeders/tests/regression_h3.rs` — **regression test**. Boots seeder against wiremock OpenSky; asserts no HTTP server is started by the seeder; asserts upstream call count = 1 per cycle. Reverting to HTTP-loopback (the temporary regression) breaks the assertion.
- E2E: at relay level.

**Verify:**

```bash
cargo nextest run -p pellucid-seeders --test regression_h3
```

### T3.8 — High-priority seeders (30 of ~140) [rust]
**Deps:** T3.5, T3.6
**Files:** `crates/pellucid-seeders/src/{markets,aviation,climate,conflict,energy}/*.rs` — at least these:
- `markets/seed_market_quotes.rs`, `seed_commodity_quotes.rs`, `seed_crypto_quotes.rs`, `seed_etf_flows.rs`, `seed_gold_etf_flows.rs`, `seed_cot.rs`
- `aviation/seed_aviation_status.rs`, `seed_notam.rs`, `seed_gpsjam.rs`
- `climate/seed_climate_anomalies.rs`, `seed_fire_detections.rs`, `seed_earthquakes.rs`, `seed_natural_events.rs`, `seed_air_quality.rs`
- `conflict/seed_ucdp_events.rs`, `seed_gdelt_intel.rs`, `seed_iran_events.rs`, `seed_unrest_events.rs`, `seed_acled.rs`
- `energy/seed_fuel_prices.rs`, `seed_jodi.rs`, `seed_spr_policies.rs`, `seed_iea_oil_stocks.rs`, `seed_gie_gas_storage.rs`, `seed_oil_inventories.rs`
- `infra/seed_internet_outages.rs`, `seed_security_advisories.rs`
- `intel/seed_telegram_intel_min.rs` (limited; full Telegram in M3 week 14)
- `prediction/seed_polymarket.rs`, `seed_forecasts.rs`

(30 total, matching spec §28 M2.)
**Deliverable:** Each seeder calls `atomic_publish` with a real envelope built from real upstream call.
**Tests (per seeder, all 30):**
- Unit: envelope shape, transformation logic.
- Integration: `crates/pellucid-seeders/tests/<seeder_name>.rs` — wiremock the upstream; full seeder run; assert SQLite contains expected envelope.
- E2E: at relay binary level T3.10.

**Verify:**

```bash
cargo nextest run -p pellucid-seeders
```

### T3.9 — C1 FIX — Relay startup gate [rust][fix]
**Deps:** T3.8
**Files:** `crates/pellucid-relay-bin/src/{main.rs,startup_check.rs}`
**Deliverable:** Per spec §17.8: hard-fail if `RELAY_SHARED_SECRET` unset in production. `ALLOW_UNAUTHENTICATED_RELAY=true` refuses to coexist with `FLY_APP_NAME` / `RAILWAY_PROJECT_ID` / `PELLUCID_PROD=true`.
**Tests:**
- Unit: `startup_check.rs` mod tests — every (secret, allow_unauth, prod_indicator) tuple → expected outcome.
- Integration: `crates/pellucid-relay-bin/tests/regression_c1.rs` — **regression test**. Boots binary with each tuple, asserts startup vs panic. Reverting the check (`if (!RELAY_SHARED_SECRET) return true;`) makes the test fail.
- E2E: `e2e/relay/startup.spec.ts` — CI deploys a relay container without secret + with `FLY_APP_NAME=test`; asserts container exits non-zero within 5 s.

**Verify:**

```bash
cargo nextest run -p pellucid-relay-bin --test regression_c1
```

### T3.10 — `pellucid-relay-bin` main + L3 FIX [rust][fix]
**Deps:** T3.1, T3.2, T3.3, T3.4, T3.6, T3.7, T3.8, T3.9
**Files:** `crates/pellucid-relay-bin/src/{main.rs,health.rs,metrics.rs,proxy/mod.rs,proxy/opensky.rs}`
**Deliverable:** Binary boots all stream tasks, scheduler with 30 seeders, Axum server on `:3004` with `/health` (real handler — **L3 fix**), `/metrics`, `/opensky` proxy.
**Tests:**
- Unit: every helper.
- Integration: `crates/pellucid-relay-bin/tests/boot.rs` — boot binary in test mode (in-memory SQLite, wiremock'd upstreams), curl `/health`, assert 200 with cascade summary.
- E2E: `e2e/relay/health.spec.ts` — Dockerfile healthcheck integration; container running, healthcheck returns 200.

**Verify:**

```bash
cargo nextest run -p pellucid-relay-bin --test boot
docker build -f docker/Dockerfile.relay -t pellucid-relay-test .
docker run -d --name relay-test --health-cmd "wget -qO- http://localhost:3004/health" pellucid-relay-test
sleep 30 && docker inspect --format='{{.State.Health.Status}}' relay-test  # healthy
docker rm -f relay-test
```

### T3.11 — Bootstrap returns 30 hydrated keys [webview][rust]
**Deps:** T2.7, T3.10
**Files:** Updates `crates/pellucid-handlers/src/bootstrap/keys.rs` with the real key list now that 30 seeders write to them.
**Deliverable:** `/api/bootstrap?tier=fast` returns 30 of 67 keys hydrated. UI shows real seeded data in the corresponding panels (still skeleton panels until M3, but the data is there).
**Tests:**
- Unit: keys.rs constants completeness check.
- Integration: `crates/pellucid-handlers/tests/bootstrap.rs` (extended) — after relay runs once, fast tier returns ≥ 30 keys hydrated.
- E2E: `e2e/boot/hydration.spec.ts` — full stack test: relay seeded SQLite, edge serves bootstrap, webview hydrates, panels show non-empty state.

**Verify:**

```bash
cargo nextest run -p pellucid-handlers --test bootstrap
bunx playwright test e2e/boot/hydration.spec.ts
```

### T3.12 — Fly.io deployment manifests [setup]
**Deps:** T3.10, T2.6
**Files:** `deploy/fly/edge.toml`, `deploy/fly/relay.toml`, `docker/Dockerfile.edge`, `docker/Dockerfile.relay`
**Deliverable:** Two Fly apps configured (`pellucid-edge`, `pellucid-relay`). Multi-arch images build via `cargo zigbuild`. `flyctl deploy` succeeds for both.
**Tests:**
- Unit: TOML parses (`fly config validate`).
- Integration: deploy to Fly **staging** environment via CI; smoke test hits edge `/api/health` and relay `/health`.
- E2E: `e2e/staging/smoke.spec.ts` — once per CI run, against staging URL.

**Verify:**

```bash
flyctl config validate -c deploy/fly/edge.toml
flyctl config validate -c deploy/fly/relay.toml
docker build -f docker/Dockerfile.edge -t pellucid-edge:test .
docker build -f docker/Dockerfile.relay -t pellucid-relay:test .
```

### M2 Gate [gate]

Spec §28 M2 exit: relay deployed to Fly.io; 30 cache keys populated; bootstrap returns hydrated data; C1, H3 regression tests green.

```bash
just check
cargo nextest run --workspace
bunx playwright test
flyctl status -a pellucid-relay-staging   # must be healthy
flyctl status -a pellucid-edge-staging    # must be healthy
```

---

## Milestone 3 — Panels (Spec §28 M3; week 10–18)

**Goal:** All 86 panels rendering with real data on all 5 variants. Hybrid task structure: one family task (T4.x.0) plus per-panel sub-tasks (T4.x.<n>).

For every panel sub-task, the deliverable is:

- A `webview/src/panels/<family>/<PanelName>.tsx` React component that uses `<Panel>` from T1.6.
- A data loader entry `webview/src/data/loaders/<family>/<rpc-name>.ts`.
- The corresponding Rust handler under `crates/pellucid-handlers/src/<domain>/v1/<rpc>.rs` (if not already shipped by an M2 seeder).
- One or more seeders if the panel needs server-side seeded data.
- Unit + integration + e2e tests (universal mandate). E2E always includes a visual-regression golden screenshot per variant where the panel is enabled.

### Family 4.1 — News / intel (week 10, 8 panels)

#### T4.1.0 — News/intel family scaffold [webview]
**Deps:** M2 Gate
**Files:** `webview/src/panels/news/index.ts` (registry), shared sub-components `NewsCard`, `IntelEntityChip`, `BreakingTicker`, `SignalSeverityBadge`.
**Tests:** Unit per shared component; integration mounting two panels side-by-side; e2e family showcase page.

#### T4.1.1 — `NewsPanel` [panel]
Files: `webview/src/panels/news/NewsPanel.tsx`, `webview/src/data/loaders/news/list.ts`, handler `news/v1/list-articles.rs`.
Tests: unit (states + filtering), integration (data loader + store), e2e on web + desktop, visual golden per variant where enabled.

#### T4.1.2 — `LiveNewsPanel` [panel]
Files: `webview/src/panels/news/LiveNewsPanel.tsx`, loader `news/live.ts`, handler `news/v1/list-live.rs`.
Tests: full set; e2e includes WebSocket-equivalent SSE handshake.

#### T4.1.3 — `BreakingNewsBanner` [panel]
Files: `webview/src/panels/news/BreakingNewsBanner.tsx` (Radix `Toast`), handler `news/v1/get-breaking.rs`.
Tests: full set; e2e asserts toast auto-dismiss + click-through.

#### T4.1.4 — `GdeltIntelPanel` [panel]
Files: `webview/src/panels/intel/GdeltIntelPanel.tsx`, loader `intel/gdelt.ts`, handler `intelligence/v1/gdelt-feed.rs`.

#### T4.1.5 — `TelegramIntelPanel` [panel]
Files: `webview/src/panels/intel/TelegramIntelPanel.tsx`, loader `intel/telegram.ts`, handler `telegram/v1/feed.rs`. Telegram seeder graduates to full implementation here.

#### T4.1.6 — `RegionalIntelligenceBoard` [panel]
Files: `webview/src/panels/intel/RegionalIntelligenceBoard.tsx`, loader `intel/regional.ts`, handler `intelligence/v1/regional.rs`.

#### T4.1.7 — `CountryDeepDivePanel` [panel]
Files: `webview/src/panels/intel/CountryDeepDivePanel.tsx`, loader `intel/country-deep-dive.ts`, handler `intelligence/v1/country-deep-dive.rs`.

#### T4.1.8 — `CountryBriefPanel` [panel]
Files: `webview/src/panels/intel/CountryBriefPanel.tsx`, loader `intel/country-brief.ts`, handler `intelligence/v1/country-brief.rs`.

**Verify (family):**

```bash
cargo nextest run -p pellucid-handlers --test news --test intelligence --test telegram
bun test webview/src/panels/news webview/src/panels/intel
bunx playwright test e2e/panels/news e2e/panels/intel
```

### Family 4.2 — Markets / finance (week 11, 12 panels)

#### T4.2.0 — Markets family scaffold [webview]
Shared: `OhlcChart`, `MetricGrid`, `SymbolPicker`, `WatchlistRow`. Tests as universal mandate.

#### T4.2.1 — `MarketPanel` [panel]
Loader + handler `market/v1/list-market-quotes.rs` (already seeded T3.8).

#### T4.2.2 — `StockAnalysisPanel` [panel]
Loader + handler `market/v1/analyze-stock.rs` (tier 2 — gate enforced).

#### T4.2.3 — `StockBacktestPanel` [panel]
Handler `market/v1/backtest-stock.rs` (tier 2).

#### T4.2.4 — `MarketBreadthPanel` [panel]
Handler `market/v1/breadth.rs`.

#### T4.2.5 — `ETFFlowsPanel` [panel]
Handler `market/v1/etf-flows.rs` (already seeded).

#### T4.2.6 — `FearGreedPanel` [panel]
Handler `market/v1/fear-greed.rs`.

#### T4.2.7 — `CotPositioningPanel` [panel]
Handler `market/v1/cot.rs` (already seeded).

#### T4.2.8 — `EarningsCalendarPanel` [panel]
Handler `market/v1/earnings.rs`.

#### T4.2.9 — `YieldCurvePanel` [panel]
Handler `market/v1/yield-curve.rs`.

#### T4.2.10 — `StablecoinPanel` [panel]
Handler `market/v1/stablecoins.rs`.

#### T4.2.11 — `LiquidityShiftsPanel` [panel]
Handler `market/v1/liquidity-shifts.rs`.

#### T4.2.12 — `DailyMarketBriefPanel` [panel]
Handler `market/v1/daily-brief.rs` (relies on ML summarization stub until M4; here, tests use a recorded summary fixture, then re-test against M4 ML once available).

**Verify (family):**

```bash
cargo nextest run -p pellucid-handlers --test market
bun test webview/src/panels/markets
bunx playwright test e2e/panels/markets
```

### Family 4.3 — Macro / economy (week 12, 11 panels)

#### T4.3.0 — Macro family scaffold [webview]
Shared: `EconIndicatorTile`, `CpiBreakdown`, `CountryEconCard`.

#### T4.3.1 — `EconomicPanel`
Handler `economic/v1/snapshot.rs`.

#### T4.3.2 — `ConsumerPricesPanel`
Handler `consumer-prices/v1/list.rs`.

#### T4.3.3 — `FSIPanel`
Handler `economic/v1/financial-stress-index.rs`.

#### T4.3.4 — `MacroSignalsPanel`
Handler `economic/v1/macro-signals.rs`.

#### T4.3.5 — `MacroTilesPanel`
Handler `economic/v1/macro-tiles.rs`.

#### T4.3.6 — `NationalDebtPanel`
Handler `economic/v1/national-debt.rs`.

#### T4.3.7 — `BigMacPanel`
Handler `economic/v1/big-mac.rs`.

#### T4.3.8 — `GroceryBasketPanel`
Handler `consumer-prices/v1/grocery-basket.rs`.

#### T4.3.9 — `FuelPricesPanel`
Handler `economic/v1/fuel-prices.rs` (seeded).

#### T4.3.10 — `FaoFoodPriceIndexPanel`
Handler `economic/v1/fao-food-price-index.rs`.

#### T4.3.11 — `GulfEconomiesPanel`
Handler `economic/v1/gulf-economies.rs`.

**Verify (family):** parallel to 4.2.

### Family 4.4 — Energy / commodities (week 13, 6 panels)

#### T4.4.0 — Energy family scaffold

#### T4.4.1 — `EnergyComplexPanel`
Handler `eia/v1/energy-complex.rs`.

#### T4.4.2 — `EnergyCrisisPanel`
Handler `eia/v1/crisis-indicators.rs`.

#### T4.4.3 — `OilInventoriesPanel`
Handler `eia/v1/oil-inventories.rs` (seeded).

#### T4.4.4 — `HormuzPanel`
Handler `maritime/v1/hormuz.rs`. Uses AIS stream + chokepoint registry.

#### T4.4.5 — `RenewableEnergyPanel`
Handler `eia/v1/renewables.rs`.

#### T4.4.6 — `GoldIntelligencePanel`
Handler `market/v1/gold-intel.rs`.

### Family 4.5 — Geopolitics / military (week 14, 10 panels) — Telegram graduates

#### T4.5.0 — Geo family scaffold + Telegram full impl
Telegram MTProto graduates from minimal to full (`pellucid-streams::telegram::run`). Channel set persisted in vault. Encrypted-at-rest session.

#### T4.5.1 — `UcdpEventsPanel`
Handler `conflict/v1/ucdp-events.rs` (seeded).

#### T4.5.2 — `StrategicPosturePanel`
Handler `military/v1/strategic-posture.rs` (seeded as theater-posture).

#### T4.5.3 — `StrategicRiskPanel`
Handler `military/v1/strategic-risk.rs`.

#### T4.5.4 — `MilitaryCorrelationPanel`
Handler `military/v1/correlation.rs` — uses correlation engine (M4 dependency; fixture data until M4).

#### T4.5.5 — `EscalationCorrelationPanel`
Handler `military/v1/escalation-correlation.rs`.

#### T4.5.6 — `ThermalEscalationPanel`
Handler `thermal/v1/escalation.rs`.

#### T4.5.7 — `DefensePatentsPanel`
Handler `military/v1/defense-patents.rs`.

#### T4.5.8 — `SanctionsPressurePanel`
Handler `sanctions/v1/pressure.rs`.

#### T4.5.9 — `SupplyChainPanel`
Handler `supply-chain/v1/risk.rs`.

#### T4.5.10 — `TradePolicyPanel`
Handler `trade/v1/policy.rs`.

### Family 4.6 — Climate / nature (week 15, 7 panels)

#### T4.6.0 — Climate family scaffold

#### T4.6.1 — `ClimateAnomalyPanel` (handler `climate/v1/anomalies.rs` seeded)
#### T4.6.2 — `ClimateNewsPanel` (handler `climate/v1/news.rs`)
#### T4.6.3 — `DisasterCorrelationPanel` (handler `natural/v1/disaster-correlation.rs`)
#### T4.6.4 — `SatelliteFiresPanel` (handler `natural/v1/fires.rs` seeded)
#### T4.6.5 — `RadiationWatchPanel` (handler `radiation/v1/watch.rs`)
#### T4.6.6 — `DiseaseOutbreaksPanel` (handler `health/v1/outbreaks.rs`)
#### T4.6.7 — `SpeciesComebackPanel` (handler `positive-events/v1/species-comeback.rs`)

### Family 4.7 — Infra / cyber (week 16, 5 panels)

#### T4.7.0 — Infra/cyber family scaffold

#### T4.7.1 — `InternetDisruptionsPanel` (handler `infrastructure/v1/disruptions.rs` seeded)
#### T4.7.2 — `SecurityAdvisoriesPanel` (handler `cyber/v1/advisories.rs` seeded)
#### T4.7.3 — `ServiceStatusPanel` (handler `infrastructure/v1/service-status.rs`)
#### T4.7.4 — `CIIPanel` (handler `infrastructure/v1/cii-score.rs` — uses scoring port)
#### T4.7.5 — `CommunityWidget` (handler `community/v1/widget.rs`)

### Family 4.8 — Forecast / prediction (week 17, 5 panels)

#### T4.8.0 — Forecast family scaffold

#### T4.8.1 — `ForecastPanel` (handler `forecast/v1/get.rs`)
#### T4.8.2 — `PredictionPanel` (handler `prediction/v1/markets.rs` — Polymarket etc., seeded)
#### T4.8.3 — `DeductionPanel` (handler `forecast/v1/deduction.rs`)
#### T4.8.4 — `CrossSourceSignalsPanel` (handler `intelligence/v1/cross-source-signals.rs`)
#### T4.8.5 — `CorrelationPanel` (handler `intelligence/v1/correlation.rs` — depends on M4 correlation engine; tests use fixture until M4)

### Family 4.9 — Chat / MCP / modals / utilities / auth-billing (week 18, 22 components)

#### T4.9.0 — Family scaffold
**Files:** Shared sub-components `<ChatTranscript>`, `<ToolCallCard>`, `<EntityIndexBadge>`.

#### T4.9.1 — `ChatAnalystPanel` [panel]
Handler `intelligence/v1/chat.rs`. Streaming response via SSE.

#### T4.9.2 — `WidgetChatModal` [panel]
Reuses chat handler.

#### T4.9.3 — `McpConnectModal` [panel]
Handler `mcp/v1/connect.rs`. MCP protocol bridge.

#### T4.9.4 — `McpDataPanel` [panel]
Handler `mcp/v1/data.rs`.

#### T4.9.5 — `SignalModal` [panel]
Radix Dialog wrapping signal detail.

#### T4.9.6 — `StoryModal` [panel]
Radix Dialog wrapping story expansion. Handler `story/v1/get.rs`.

#### T4.9.7 — `SearchModal` [panel]
Radix Dialog with `searchManager`. Handler `search/v1/global.rs`.

#### T4.9.8 — `CountryIntelModal` [panel]
Reuses country intel handlers.

#### T4.9.9 — `MobileWarningModal` [panel]
Static Radix Dialog; UA detection.

#### T4.9.10 — `UnifiedSettings` [panel]
Settings dialog with all preference categories.

#### T4.9.11 — `VirtualList` [panel]
Reusable wrapper around `react-virtuoso`.

#### T4.9.12 — `IntelligenceGapBadge` [panel]
Component + handler `intelligence/v1/gap-summary.rs`.

#### T4.9.13 — `LlmStatusIndicator` [panel]
Reads `pellucid-ml` health (M4 dep; fixture until M4).

#### T4.9.14 — `AuthHeaderWidget` [panel]
Clerk integration in webview.

#### T4.9.15 — `AuthLauncher` [panel]
Tauri-aware (deep-link handler for desktop OAuth callback).

#### T4.9.16 — `ProBanner` [panel]
Driven by `useAuthStore.entitlements`.

#### T4.9.17 — `DownloadBanner` [panel]
Hosted-only (renders only on web build).

#### T4.9.18 — `payment-failure-banner` [panel]
Toast triggered by Convex webhook → entitlement update.

#### T4.9.19 — `PlaybackControl` [panel]
Map playback timeline (Radix Slider). Spec §3 Maps.

#### T4.9.20 — `MapContextMenu` [panel]
Radix Popover.

#### T4.9.21 — `MapPopup` [panel]
Radix Popover anchored to map features.

#### T4.9.22 — Audit residual components
**Action:** Owner reviews `webview/src/panels/**` against the spec's "all 86 panels" mandate. If any panel listed in the original WorldMonitor `LoreWorldMonitorComponents.md §3` is missing, file a sub-task here. Owner: M3 lead. Deadline: M3 week 18 close.

### M3 Gate [gate]
**Verify:**

```bash
just check
bun test webview/src/panels
bunx playwright test e2e/panels e2e/visual
cargo nextest run -p pellucid-handlers
```

Spec §28 M3 exit: all 86 panels rendering with real data on all 5 variants; visual regression deltas ≤ 0.5 %; full universal-mandate coverage (every component has unit + integration + e2e).

---

## Milestone 4 — ML + correlation (Spec §28 M4; week 19–21)

**Goal:** `pellucid-ml` with `ort` backend + 4 bundled models; `pellucid-correlation` with 4 adapters; embeddings + sqlite-vec semantic search; webview correlation panel + signal modal use real Rust IPC.

### T5.1 — `pellucid-ml` skeleton + ort wiring [rust]
**Deps:** M3 Gate
**Files:** `crates/pellucid-ml/{Cargo.toml,src/lib.rs,src/engine.rs,src/ort_engine.rs,src/candle_engine.rs,src/models/mod.rs}`
**Deliverable:** `MlEngine` trait + `OrtEngine` impl. `candle` impl behind feature flag. Models loaded at startup from configured path.
**Tests:**
- Unit: trait conformance; engine selection.
- Integration: `crates/pellucid-ml/tests/engine.rs` — load tiny test model, run inference, assert deterministic output.
- E2E: at handler level T5.4.

**Verify:**

```bash
cargo nextest run -p pellucid-ml --test engine
```

### T5.2 — Bundled models + Tauri resource packaging [rust]
**Deps:** T5.1
**Files:** `crates/pellucid-ml/models/{minilm-l6.onnx,distilbert-sst2.onnx,bart-cnn-summary.onnx,xlm-roberta-ner.onnx}` + `tauri.conf.json` resources entry.
**Deliverable:** Models bundled into Tauri resource directory; edge binary downloads on first use (configurable URL; SHA256 verified).
**Tests:**
- Unit: `model_paths.rs` mod tests.
- Integration: `crates/pellucid-ml/tests/load_real_models.rs` — load each model, run a known input through it, assert output shape (and for sentiment, output class for a known sentence).
- E2E: at panel level.

**Verify:**

```bash
cargo nextest run -p pellucid-ml --test load_real_models
```

### T5.3 — Embed / sentiment / summarize / extract_entities [rust]
**Deps:** T5.2
**Files:** `crates/pellucid-ml/src/{embed.rs,sentiment.rs,summarize.rs,ner.rs}`
**Deliverable:** All four operations + `batch_embed`. Tokenization via `tokenizers` crate (HF).
**Tests:**
- Unit: each function with golden inputs/outputs from a Python reference.
- Integration: `crates/pellucid-ml/tests/{embed,sentiment,summarize,ner}.rs` — golden fixtures from original transformers.js outputs (recorded once); diff tolerance ε = 1e-3 for embeddings.
- E2E: at handler level.

**Verify:**

```bash
cargo nextest run -p pellucid-ml
```

### T5.4 — `pellucid-handlers` intelligence/v1 ML endpoints [rust]
**Deps:** T5.3
**Files:** `crates/pellucid-handlers/src/intelligence/v1/{extract_entities,search_semantic,summarize_article,classify_event}.rs`
**Deliverable:** Real handlers replacing fixture data used by M3 panels.
**Tests:**
- Unit per handler.
- Integration `crates/pellucid-handlers/tests/intelligence.rs` — full pipeline: gateway + handler + cache + ML. Cold cache → ML inference. Warm cache → no inference.
- E2E `e2e/panels/intel-semantic.spec.ts` — search produces matching results.

**Verify:**

```bash
cargo nextest run -p pellucid-handlers --test intelligence
bunx playwright test e2e/panels/intel-semantic.spec.ts
```

### T5.5 — `pellucid-correlation` engine [rust]
**Deps:** T5.3, T1.3
**Files:** `crates/pellucid-correlation/src/{lib.rs,trait.rs,jaccard.rs,adapters/{military,escalation,economic,disaster}.rs}`
**Deliverable:** All four adapters from spec §11.2. Cross-domain Jaccard clustering port of `analysis.worker.ts`.
**Tests:**
- Unit per adapter; Jaccard property test (symmetric, transitive bounds).
- Integration `crates/pellucid-correlation/tests/full.rs` — fixture inputs from original `tests/clustering.test.mjs`; output matches within tolerance.
- E2E at panel level.

**Verify:**

```bash
cargo nextest run -p pellucid-correlation
```

### T5.6 — sqlite-vec semantic search wired [rust]
**Deps:** T5.3, T1.2
**Files:** `crates/pellucid-handlers/src/news/v1/search-semantic.rs`, `crates/pellucid-seeders/src/news/embed_articles.rs`
**Deliverable:** New seeder embeds article bodies, writes to `embeddings`. Handler queries via `MATCH … k=20`.
**Tests:**
- Unit: query construction.
- Integration: index 50 fixture articles, query, assert top-1 match.
- E2E: `e2e/panels/semantic-search.spec.ts`.

**Verify:**

```bash
cargo nextest run -p pellucid-handlers --test news
bunx playwright test e2e/panels/semantic-search.spec.ts
```

### T5.7 — Webview IPC for ML (desktop) [webview][rust]
**Deps:** T5.3, T1.7
**Files:** Tauri commands `ml_embed`, `ml_sentiment`, `ml_summarize`, `ml_extract_entities`. Webview wrapper `webview/src/services/ml.ts`.
**Deliverable:** Desktop bypasses HTTP → invokes ML directly via IPC; web continues to use `/api/intelligence/*`.
**Tests:**
- Unit per IPC command.
- Integration: `crates/pellucid-tauri/tests/ipc_ml.rs` (real ML engine, simple prompt).
- E2E: `e2e/desktop/ml.spec.ts`.

**Verify:**

```bash
cargo nextest run -p pellucid-tauri --test ipc_ml
bunx playwright test --project=desktop e2e/desktop/ml.spec.ts
```

### T5.8 — M3 fixture-data panels graduate to real ML [webview]
**Deps:** T5.4, T5.5, T5.6, T5.7
**Files:** Updates panels that used fixtures in M3 (T4.2.12, T4.5.4, T4.5.5, T4.8.5, T4.9.13).
**Deliverable:** `DailyMarketBriefPanel`, `MilitaryCorrelationPanel`, `EscalationCorrelationPanel`, `CorrelationPanel`, `LlmStatusIndicator` all use real backends.
**Tests:** Re-run T4.x tests with real backends; new e2e specs assert real-data behavior on each.

**Verify:**

```bash
bun test webview/src/panels
bunx playwright test e2e/panels
```

### M4 Gate [gate]
Spec §28 M4 exit: `intelligence/v1/extract-entities`, `news/v1/search-semantic`, `correlation/v1/run` return parity-quality results vs source product.

```bash
just check
cargo nextest run --workspace
bunx playwright test
```

---

## Milestone 5 — Hardening + remaining fixes (Spec §28 M5; week 22–24)

**Goal:** Every Medium/Low fix from spec §24.3, §24.4. Performance pass. Visual regression sweep.

### T6.1 — M2 FIX — Per-host IPv4 allowlist [rust][fix]
**Deps:** M4
**Files:** `data/ipv4-required-hosts.json`, `crates/pellucid-streams/src/http_client.rs` (wraps reqwest with per-host `local_address`).
**Deliverable:** Default dual-stack; only hosts listed get IPv4-forced. Replaces global IPv4 monkey-patch.
**Tests:**
- Unit: hostname matcher.
- Integration: `crates/pellucid-streams/tests/regression_m2.rs` — call to allowlisted host uses IPv4; non-allowlisted uses dual-stack.
- E2E: relay smoke against staging.

**Verify:**

```bash
cargo nextest run -p pellucid-streams --test regression_m2
```

### T6.2 — M3 FIX — Seeder skip metrics [rust][fix]
**Deps:** M2
**Files:** `crates/pellucid-seeders/src/metrics.rs` (extends T3.6).
**Deliverable:** `metrics::counter!("seeder_skip_transient", "domain" => …, "reason" => …)`. Surfaced at `/metrics` on relay binary.
**Tests:**
- Unit: counter increments on simulated transient error.
- Integration: `crates/pellucid-seeders/tests/regression_m3.rs` — wiremock returns 503; assert counter incremented.
- E2E: hit `/metrics` on relay, parse Prometheus output, assert metric present.

**Verify:**

```bash
cargo nextest run -p pellucid-seeders --test regression_m3
curl http://localhost:3004/metrics | grep seeder_skip_transient
```

### T6.3 — M5 FIX — Single-source CSP [setup][fix]
**Deps:** T0.10
**Files:** `tools/build-csp.ts` graduates to full implementation; `tools/check-csp.ts` parity tester.
**Deliverable:** Single source; pre-push fails on divergence.
**Tests:**
- Unit per tool function.
- Integration `tools/check-csp.test.ts` — fixtures: aligned (pass), one diverged (fail).
- E2E: regression test in `crates/pellucid-edge-bin/tests/csp_parity.rs` — runs build-csp, reads three locations, asserts equal.

**Verify:**

```bash
bun test tools/check-csp.test.ts tools/build-csp.test.ts
cargo nextest run -p pellucid-edge-bin --test csp_parity
```

### T6.4 — M6 FIX — Envelope path consolidation [rust][fix]
**Deps:** M2
**Files:** `crates/pellucid-seeders/src/envelope.rs` mandates enveloped output by default; bare path emits `tracing::warn!` and increments a `bare_seed_publish` counter.
**Deliverable:** Roadmap entry for v1.1 cutoff documented in this plan §9 (Future Work). All M2/M3 seeders confirmed enveloped.
**Tests:**
- Unit: `envelope.rs` mod tests — `publish_bare` warns; `publish_enveloped` does not.
- Integration: `crates/pellucid-seeders/tests/regression_m6.rs` — every seeder in registry; assert all use enveloped.

**Verify:**

```bash
cargo nextest run -p pellucid-seeders --test regression_m6
```

### T6.7 — M8 FIX — Aggregate rate-limit cap [rust][fix]
**Deps:** T1.4
**Files:** Already added in T1.4 — verified here.
**Deliverable:** Regression test promoted to gate.
**Tests:**
- Integration `crates/pellucid-cache/tests/regression_m8.rs` — exhausts an endpoint bucket and a global bucket; aggregate cap blocks first.
- E2E: not applicable (low-level concern).

**Verify:**

```bash
cargo nextest run -p pellucid-cache --test regression_m8
```

### T6.8 — M9 FIX — Vault keychain-change listener [rust][fix]
**Deps:** T1.7
**Files:** `crates/pellucid-tauri/src/vault_listener/{macos.rs,windows.rs,linux.rs}` (platform-specific).
**Deliverable:** Listener per platform; on event, invokes `refresh_secrets`. Handler retry path verified.
**Tests:**
- Unit per platform — mock notification.
- Integration `crates/pellucid-tauri/tests/regression_m9.rs` — simulated keychain change; assert refresh_secrets fired within 1 s.
- E2E: `e2e/desktop/vault-rotate.spec.ts` (CI macOS-only) — modify test keychain entry, assert app picks up new value.

**Verify:**

```bash
cargo nextest run -p pellucid-tauri --test regression_m9
bunx playwright test --project=desktop e2e/desktop/vault-rotate.spec.ts
```

### T6.9 — Performance pass [gate]
**Deps:** all prior M5
**Files:** `tools/perf/{bootstrap-bench.ts,gateway-bench.rs,cache-bench.rs}`
**Deliverable:** Bootstrap p95 ≤ 800 ms (edge), ≤ 200 ms (sidecar). Cache stampede coalesce ratio ≥ 0.99 under 1000-concurrent test.
**Tests:**
- Bench: `cargo bench --workspace`.
- Integration: `tools/perf/bootstrap-bench.ts` runs 200 cold-cache invocations; reports p95.
- E2E: not applicable.

**Verify:**

```bash
cargo bench --workspace
bun run tools/perf/bootstrap-bench.ts
```

### T6.10 — Visual regression sweep [gate]
**Deps:** M3
**Files:** `e2e/visual/**` — every panel × every variant captured.
**Deliverable:** All visual deltas ≤ 0.5 %.
**Verify:**

```bash
bunx playwright test e2e/visual --update-snapshots=missing
bunx playwright test e2e/visual
```

### T6.11 — Audit pass [gate]
**Files:** none new.
**Verify:**

```bash
cargo audit
bun audit
cargo deny check
```

### M5 Gate [gate]
Spec §28 M5 exit: all Quality Gates §23 pass. Reviewer-signed completion checklist filed.

```bash
just check
cargo nextest run --workspace
bunx playwright test
cargo bench --workspace
cargo audit && bun audit
```

---

## Milestone 6 — GA (Spec §28 M6; week 25)

### T7.1 — Production deploy [setup]
**Deps:** M5
**Files:** `deploy/fly/edge-prod.toml` (3 regions: iad, fra, syd), `deploy/fly/relay-prod.toml` (single region with secondary failover).
**Deliverable:** Production Fly apps live. Litestream replicating to Cloudflare R2 every 60 s.
**Tests:**
- Integration: `e2e/staging/full.spec.ts` against staging URL passes.
- E2E: `e2e/prod/smoke.spec.ts` — read-only checks against production health endpoints.
**Verify:**

```bash
flyctl deploy -c deploy/fly/edge-prod.toml --image-label v1.0.0
flyctl deploy -c deploy/fly/relay-prod.toml --image-label v1.0.0
bunx playwright test e2e/prod/smoke.spec.ts
```

### T7.2 — Signed desktop releases [setup]
**Deps:** M5
**Files:** `.github/workflows/release-desktop.yml` (signed builds, GitHub Releases publish).
**Deliverable:** macOS (notarized arm64 + x86_64), Windows (EV-signed), Linux (.deb / .rpm / AppImage with GPG).
**Tests:**
- Integration: `tools/test-installers.sh` runs each installer in a VM and asserts app launches.
- E2E: `e2e/desktop/installer.spec.ts` per platform.
**Verify:**

```bash
gh release view v1.0.0   # all artifacts present
bash tools/test-installers.sh
```

### T7.3 — DNS cutover [setup]
**Deps:** T7.1
**Files:** DNS records + Fly certs.
**Deliverable:** `worldmonitor.app` and `api.worldmonitor.app` (and variant subdomains) point at Pellucid.
**Tests:** `e2e/dns-cutover.spec.ts` polls all expected hostnames; asserts certificate chain.
**Verify:**

```bash
dig worldmonitor.app
dig api.worldmonitor.app
bunx playwright test e2e/dns-cutover.spec.ts
```

### T7.4 — Public API docs [setup][rust]
**Deps:** M5
**Files:** `docs.worldmonitor.app` Mintlify project regenerated from generated OpenAPI; `crates/pellucid-codegen` emits OpenAPI v3 from sebuf.
**Deliverable:** Documented endpoints live and discoverable.
**Tests:**
- Unit per generator function.
- Integration: produced OpenAPI validates with `swagger-cli validate`.
- E2E: `e2e/docs/links.spec.ts` — fetches `docs.worldmonitor.app`, asserts every documented endpoint resolves.
**Verify:**

```bash
cargo run -p pellucid-codegen -- openapi --out docs.worldmonitor.app/openapi.json
bunx swagger-cli validate docs.worldmonitor.app/openapi.json
bunx playwright test e2e/docs/links.spec.ts
```

### T7.5 — Acceptance evidence bundle [gate]
**Deps:** all prior
**Files:** `docs/release/v1.0.0-acceptance.md`
**Deliverable:** Per CLAUDE.md and spec §31, the evidence bundle:

1. Each of 23 OP rows linked to passing test.
2. Each of 14 quality gates with command + output tail.
3. Each of C1, H1–H4 regression tests with revert-confirms-failure log.
4. Bootstrap p95 measurement.
5. Health endpoint 60 s green log.
6. CSP triplication parity log.
7. `cargo audit` + `bun audit` clean output.
8. `git diff --stat` of v0 → v1 commit.
9. Typecheck + lint + clippy clean output.
10. Explicit deferred-items list (initially empty for any open spec items).

**Verify (final):**

```bash
just check
cargo nextest run --workspace
bunx playwright test
cargo bench --workspace
cargo audit && bun audit
cat docs/release/v1.0.0-acceptance.md
```

### M6 Gate [gate] = v1 GA
Spec §31 acceptance criteria fully satisfied with linkable evidence.

---

## 4. Milestone-spanning concerns

### 4.1 Per-task evidence file

Each task contributor commits, alongside their code, a file `docs/evidence/T<id>.md` containing:

```text
Task: T<id> — <title>
Branch / commit: <sha>
Verify commands run:
  $ <cmd>
  ...
Test output tail:
  ...
Coverage:
  unit: NN%
  integration: NN%
git diff --stat:
  ...
Notes / deferrals:
  - none | <list>
```

Pre-push hook requires this file to exist for the modified task IDs.

### 4.2 Subagent dispatch hints

Tasks suitable for parallel agent dispatch are noted in the per-task `Notes` (added during execution). Default: in M3, panel sub-tasks within a family are independent and can be dispatched in parallel via `/lore:subagent-development`. Across families, sequential because they share data-loader scaffolding.

### 4.3 Future work (post-GA, tracked here, not blocking)

- LiteFS read replicas (spec OD-4)
- Bare-envelope path removal cutoff date (M6 fix continuation)
- Enterprise tier features (spec OD-10)
- Pricing page rebuild (spec OD-8)
- Web Telegram client variant (none today)

---

## 5. Open Decisions (carried from spec §30)

These are unresolved at plan write-time. Each is owned by `User` per the spec lock and must be resolved by the deadline below.

| ID | Decision | Default | Resolve by |
|---|---|---|---|
| OD-1 | ML backend default — `ort` vs `candle` | `ort` | start of T5.1 |
| OD-2 | Hosted edge cloud — Fly.io vs Railway vs self-host | Fly.io | start of T3.12 |
| OD-3 | Litestream replica destination | Cloudflare R2 | start of T7.1 |
| OD-4 | LiteFS adoption | Deferred | post-GA |
| OD-5 | Web SPA tenancy — single Fly app vs per-variant | Single app, hostname routing | start of T3.12 |
| OD-6 | Telegram client — `grammers-client` vs `tdlib` | `grammers-client` | start of T4.5.0 |
| OD-7 | Variant `happy` retention | Keep | M3 |
| OD-8 | Pricing page rebuild | v1.1 | M5 |
| OD-9 | MCP-related panels — keep or pivot | Keep | T4.9.3 / T4.9.4 |
| OD-10 | Enterprise tier features | v1.1 | M5 |

---

## 6. Phase 4 — Execution handoff

This plan is structured for `/lore:execute` to consume directly. To begin execution:

### Option A — Inline single-session

```text
/lore:execute docs/plans/2026-04-25-pellucid-rebuild.md
```

`/lore:execute` reads the plan, picks T0.1 (no deps), runs it to completion (write code → write tests → run verify → commit), then advances to the next unblocked task. Every milestone gate is a checkpoint that can resume.

### Option B — Subagent-driven parallelism

```text
/lore:subagent-development docs/plans/2026-04-25-pellucid-rebuild.md
```

Within a milestone, the orchestrator dispatches independent tasks to fresh subagents (e.g., during M3, all panel sub-tasks within a family run in parallel). Two-stage review per task per the lore framework.

### Option C — Worktree-isolated experiment

```text
/lore:worktree start pellucid-m0
/lore:execute docs/plans/2026-04-25-pellucid-rebuild.md --through M0
```

Recommended when running early milestones so the bootstrap can be validated end-to-end before merging.

### Resume points (built into the plan)

Resume `/lore:execute` from any of these checkpoints:

- T0 Gate
- M0 Gate
- M1 Gate
- M2 Gate
- M3 Gate (and per-family sub-checkpoints T4.1–T4.9)
- M4 Gate
- M5 Gate
- M6 Gate (= v1 GA)

### Pre-execution prerequisites the user must satisfy

Before T7.1 (production deploy), the user must:

1. Provision Fly.io org + tokens; set `FLY_API_TOKEN` in CI secrets.
2. Provision Cloudflare R2 bucket; set Litestream env vars.
3. Provision Clerk production instance; set `CLERK_PUBLISHABLE_KEY` + `CLERK_SECRET_KEY`.
4. Provision Dodo production account; set `DODO_PAYMENTS_API_KEY` + `DODO_PAYMENTS_WEBHOOK_SECRET` + `DODO_IDENTITY_SIGNING_SECRET`.
5. Provision Convex production deployment; set `CONVEX_URL` + `CONVEX_SERVER_SHARED_SECRET`.
6. Provision Sentry projects (browser + Rust) and DSNs.
7. Provision relay shared secret; set `RELAY_SHARED_SECRET` (and never leave it unset — C1 fix enforces).
8. Provision Apple Developer ID + Microsoft EV cert + Linux GPG key for desktop signing (T7.2).
9. Configure DNS for `worldmonitor.app`, `api.worldmonitor.app`, `tech./finance./commodity./happy.worldmonitor.app`, `docs.worldmonitor.app`.

The plan does not pause at these — they are gathered concurrently with development.

---

*End of plan. Begin with `/lore:execute docs/plans/2026-04-25-pellucid-rebuild.md` from T0.1.*
