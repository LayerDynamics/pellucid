# Justfile — top-level orchestrator for the Pellucid polyglot workspace.
#
# `just --list` for the full menu; the universal CI gate is `just check`.

set shell := ["bash", "-cue"]
set positional-arguments

default:
    @just --list

# ─── Install / refresh dependencies ───────────────────────────────────────────
install:
    bun install
    cargo fetch
    tools/install-rust-tools.sh --check || tools/install-rust-tools.sh

# ─── Code generation (sebuf → handler types + TS client stubs) ────────────────
gen:
    @echo "[just gen] codegen lands in T0.10 follow-up + T2.5 (sebuf plugin)."
    @echo "[just gen] no-op until proto/ ships."

# ─── Dev servers ──────────────────────────────────────────────────────────────
dev-web:
    bun run --filter=@pellucid/webview dev

dev-desktop:
    cargo tauri dev

dev-edge:
    cargo run -p pellucid-edge-bin

dev-relay:
    cargo run -p pellucid-relay-bin

dev-convex:
    bun run --filter=@pellucid/convex dev

# ─── The universal CI gate ────────────────────────────────────────────────────
check: check-fmt check-clippy check-rust-tests check-rust-coverage check-typecheck check-lint check-bun-tests check-cache-keys check-csp check-edge-imports check-version-sync check-e2e-web

check-fmt:
    cargo fmt --check

check-clippy:
    cargo clippy --workspace --all-targets -- -D warnings

check-rust-tests:
    cargo nextest run --workspace

check-rust-coverage:
    @echo "[just check-rust-coverage] coverage gate enforced once T1.x crates land real source"
    cargo llvm-cov --workspace --no-report

check-typecheck:
    bun run --filter='*' typecheck

check-lint:
    bun run --filter='*' lint || true

check-bun-tests:
    bun run --filter='*' test

check-cache-keys:
    bun run tools/check-cache-keys.ts

check-csp:
    bun run tools/check-csp.ts

check-edge-imports:
    bun run tools/check-edge-imports.ts

check-version-sync:
    bun run tools/version-sync.ts

check-e2e-web:
    bunx playwright test

# ─── Builds ───────────────────────────────────────────────────────────────────
build-web variant="base":
    VITE_VARIANT={{variant}} bun run --filter=@pellucid/webview build

build-edge:
    cargo build --release -p pellucid-edge-bin

build-relay:
    cargo build --release -p pellucid-relay-bin

build-desktop:
    cargo tauri build

build-all: build-edge build-relay build-desktop
    @echo "[just build-all] ok"

# ─── Audits ───────────────────────────────────────────────────────────────────
audit:
    cargo audit
    bun audit || true

deny:
    cargo deny check

# ─── Cleanup ──────────────────────────────────────────────────────────────────
clean:
    cargo clean
    bun run --filter='*' clean || true
    rm -rf node_modules .bun playwright-report test-results
