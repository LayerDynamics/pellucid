# Railway image — pellucid-relay-bin.
#
# Two-stage build. Stage 1 compiles the workspace; stage 2 ships
# the static binary on debian-slim with `tini` as PID 1.
#
# Railway sets `PORT` at runtime; `pellucid-relay-bin`'s
# `ConfigSource::from_process` reads it via the
# `resolve_listen_addr_from_env` helper, so the binary binds
# `0.0.0.0:$PORT` automatically (see
# `crates/pellucid-relay-bin/src/config.rs`). No shell wrapper
# needed.
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

# ---- runtime ---------------------------------------------------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
 && apt-get install -y --no-install-recommends \
        ca-certificates \
        tini \
 && rm -rf /var/lib/apt/lists/*

# Match Railway Volumes' default uid:gid (0:0) is fine, but a
# dedicated non-root user is safer.
RUN useradd --uid 1000 --create-home --shell /usr/sbin/nologin pellucid \
 && mkdir -p /data \
 && chown pellucid:pellucid /data

COPY --from=builder /build/target/release/pellucid-relay-bin /usr/local/bin/pellucid-relay-bin
RUN chmod 755 /usr/local/bin/pellucid-relay-bin

USER pellucid
WORKDIR /data

# `tini` reaps zombies + forwards SIGTERM/SIGINT so the relay's
# `axum::serve` graceful-shutdown actually fires when Railway
# rolls a deploy.
ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/pellucid-relay-bin"]
