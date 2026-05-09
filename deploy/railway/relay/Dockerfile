# Railway image — pellucid-relay-bin.
#
# Three-stage build. Stage 1 compiles the workspace; stage 2 pulls
# the official Litestream binary; stage 3 ships the relay binary
# alongside Litestream on debian-slim with `tini` as PID 1.
#
# Railway sets `PORT` at runtime; `pellucid-relay-bin`'s
# `ConfigSource::from_process` reads it via the
# `resolve_listen_addr_from_env` helper, so the binary binds
# `0.0.0.0:$PORT` automatically (see
# `crates/pellucid-relay-bin/src/config.rs`). No shell wrapper
# needed for the binary itself, but the Litestream sidecar wraps
# it to continuously replicate the SQLite database to Digital
# Ocean Spaces (or any S3-compatible object store) per
# SPEC-001 §6 + §17.5.
#
# Build context is the repo root (Railway sets it from the
# service's "Root Directory"; we point that at the repo root and
# let `dockerfilePath` resolve here).

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

# Compile only the relay binary. Workspace deps that aren't
# transitively required by `pellucid-relay-bin` are skipped.
RUN cargo build --release --locked --bin pellucid-relay-bin
RUN strip /build/target/release/pellucid-relay-bin

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

# Railway provisions Volumes owned by root:root. We deliberately
# stay as root inside the container so the binary can read and
# write the volume mount at /data — Railway's container kernel
# namespace already isolates the service. (A non-root user would
# need an entrypoint shim that chowns /data each boot, which adds
# moving parts without buying isolation we don't already have.)
RUN mkdir -p /data /etc/litestream

COPY --from=builder /build/target/release/pellucid-relay-bin /usr/local/bin/pellucid-relay-bin
RUN chmod 755 /usr/local/bin/pellucid-relay-bin

COPY --from=litestream /usr/local/bin/litestream /usr/local/bin/litestream
RUN chmod 755 /usr/local/bin/litestream

# Litestream config + entrypoint script.
COPY deploy/railway/relay/litestream.yml /etc/litestream/litestream.yml
COPY deploy/railway/relay/entrypoint.sh /usr/local/bin/entrypoint.sh
RUN chmod 755 /usr/local/bin/entrypoint.sh

WORKDIR /data

# `tini` reaps zombies + forwards SIGTERM/SIGINT so the relay's
# `axum::serve` graceful-shutdown actually fires when Railway
# rolls a deploy. The entrypoint script owns the
# `litestream restore` (if applicable) → `litestream replicate
# -exec pellucid-relay-bin` lifecycle so DB state is restored
# from Digital Ocean Spaces before the binary opens its sqlx
# pool, and is replicated continuously while it serves requests.
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/entrypoint.sh"]
