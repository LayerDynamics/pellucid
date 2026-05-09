# Railway image — pellucid-edge-bin (SaaS apex + api host).
#
# Four-stage build:
# 1. `frontend`   — bun + vite build the React SPA into `webview/dist/`
#                   with `VITE_TARGET=web` so desktop-only branches
#                   are eliminated from the production bundle.
# 2. `builder`    — Rust 1.84 builds `pellucid-edge-bin`.
# 3. `litestream` — pulls the official Litestream binary so we can
#                   COPY it into the runtime image without dragging
#                   curl/dpkg packaging into the slim runtime layer.
# 4. `runtime`    — debian-slim with the binary, the SPA, and a
#                   Litestream sidecar that continuously replicates
#                   the SQLite database to S3-compatible storage
#                   (Digital Ocean Spaces / Cloudflare R2 / AWS S3).
#
# Railway sets `PORT` at runtime and the binary's
# `ConfigSource::from_process` reads it via
# `resolve_listen_addr_from_env`, so it binds `0.0.0.0:$PORT`
# automatically (see `crates/pellucid-edge-bin/src/config.rs`).

# ---- frontend --------------------------------------------------
FROM oven/bun:1 AS frontend

WORKDIR /app
COPY package.json bun.lock tsconfig.json ./
COPY webview ./webview
# Install webview deps + run vite directly (skipping `tsc -b`).
# `@types/bun` lives in the root package.json, not webview's, so a
# `cd webview && bun run build` on its own fails the tsc step. The
# production bundle doesn't need tsc — vite + the React plugin
# transpile TS via esbuild, and CI runs typecheck separately.
#
# `VITE_TARGET=web` is read by `webview/vite.config.ts:4` and baked
# into `__VITE_TARGET__` at build time. Code paths gated on
# `import.meta.env.VITE_TARGET === 'desktop'` (Tauri IPC, vault,
# updater, fetch patches) get tree-shaken out of the bundle. Without
# this, the SaaS bundle ships ~80 kB of dead desktop-runtime code
# and the boot orchestrator can briefly try to invoke
# `@tauri-apps/api/core invoke()` before falling back, producing a
# console error users would otherwise see.
ENV VITE_TARGET=web
RUN cd webview && bun install --frozen-lockfile && bunx vite build

# ---- builder ---------------------------------------------------
FROM rust:1.84-slim-bookworm AS builder

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        ca-certificates \
        pkg-config \
        libssl-dev \
        build-essential \
 && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY . .

RUN cargo build --release --locked --bin pellucid-edge-bin
RUN strip /build/target/release/pellucid-edge-bin

# ---- litestream ------------------------------------------------
# Pin a single Litestream version so a transient "latest" change
# doesn't rebuild the runtime image with a different replica
# protocol. 0.3.13 is the most recent stable as of 2026-05.
FROM debian:bookworm-slim AS litestream
ARG LITESTREAM_VERSION=0.3.13

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        ca-certificates \
        curl \
 && rm -rf /var/lib/apt/lists/*

# Litestream ships per-arch tarballs. `dpkg --print-architecture`
# returns `amd64`/`arm64` on Debian, matching the asset suffixes
# at github.com/benbjohnson/litestream/releases.
RUN set -eux; \
    arch="$(dpkg --print-architecture)"; \
    case "$arch" in \
        amd64) suffix="amd64" ;; \
        arm64) suffix="arm64" ;; \
        *) echo "unsupported arch: $arch" >&2; exit 1 ;; \
    esac; \
    curl -fsSL -o /tmp/litestream.tar.gz \
        "https://github.com/benbjohnson/litestream/releases/download/v${LITESTREAM_VERSION}/litestream-v${LITESTREAM_VERSION}-linux-${suffix}.tar.gz"; \
    tar -C /usr/local/bin -xzf /tmp/litestream.tar.gz litestream; \
    rm /tmp/litestream.tar.gz; \
    /usr/local/bin/litestream version

# ---- runtime ---------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        ca-certificates \
        tini \
 && rm -rf /var/lib/apt/lists/*

# Railway Volumes are owned by root:root; stay as root inside
# the container so SQLite at /data is writable. The container
# kernel namespace provides isolation; running non-root here
# would force an entrypoint shim that chowns /data each boot.
RUN mkdir -p /data /etc/litestream

COPY --from=builder /build/target/release/pellucid-edge-bin /usr/local/bin/pellucid-edge-bin
RUN chmod 755 /usr/local/bin/pellucid-edge-bin

COPY --from=litestream /usr/local/bin/litestream /usr/local/bin/litestream
RUN chmod 755 /usr/local/bin/litestream

# SPA bundle from the `frontend` stage — `pellucid-edge-bin` mounts
# `tower-http::ServeDir` at the router's fallback so any non-API
# request serves index.html or a static asset. Path matches
# `DEFAULT_WEBVIEW_DIST` in
# `crates/pellucid-edge-bin/src/config.rs:42`.
COPY --from=frontend /app/webview/dist /srv/webview

# Litestream config + entrypoint script.
COPY deploy/railway/edge/litestream.yml /etc/litestream/litestream.yml
COPY deploy/railway/edge/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod 755 /usr/local/bin/entrypoint.sh

WORKDIR /data

# `tini` reaps zombies and forwards SIGTERM cleanly to the
# entrypoint shell. The entrypoint script owns the
# `litestream restore` (if applicable) → `litestream replicate
# -exec pellucid-edge-bin` lifecycle so DB state is restored
# from Spaces before the binary opens its sqlx pool, and is
# replicated continuously while the binary serves requests.
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/entrypoint.sh"]
