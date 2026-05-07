# Railway image — pellucid-edge-bin.
#
# Three-stage build:
# 1. `frontend` — bun + vite build the React SPA into `webview/dist/`.
# 2. `builder`  — Rust 1.84 builds `pellucid-edge-bin`.
# 3. `runtime`  — debian-slim with both artefacts.
#
# Railway sets `PORT` at runtime and the binary's
# `ConfigSource::from_process` reads it via
# `resolve_listen_addr_from_env`, so it binds `0.0.0.0:$PORT`
# automatically (see `crates/pellucid-edge-bin/src/config.rs`).

# ---- frontend --------------------------------------------------
# Build the SPA in isolation so the Rust stage doesn't need bun /
# node toolchains. `webview/dist/` is in `.gitignore` so we cannot
# rely on a committed bundle — building it inside the image is the
# only way the runtime sees an up-to-date SPA.
FROM oven/bun:1 AS frontend

WORKDIR /app
COPY package.json bun.lock tsconfig.json ./
COPY webview ./webview
RUN cd webview && bun install --frozen-lockfile && bun run build

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
RUN mkdir -p /data

COPY --from=builder /build/target/release/pellucid-edge-bin /usr/local/bin/pellucid-edge-bin
RUN chmod 755 /usr/local/bin/pellucid-edge-bin

# SPA bundle from the `frontend` stage — `pellucid-edge-bin` mounts
# `tower-http::ServeDir` at the router's fallback so any non-API
# request serves index.html or a static asset. Path matches
# `DEFAULT_WEBVIEW_DIST` in
# `crates/pellucid-edge-bin/src/config.rs:42`.
COPY --from=frontend /app/webview/dist /srv/webview

WORKDIR /data

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/pellucid-edge-bin"]
