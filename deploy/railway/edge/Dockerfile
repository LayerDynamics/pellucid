# Railway image — pellucid-edge-bin.
#
# Two-stage build identical in shape to the relay's Dockerfile;
# only `--bin` differs. Railway sets `PORT` at runtime and the
# binary's `ConfigSource::from_process` reads it via
# `resolve_listen_addr_from_env`, so it binds `0.0.0.0:$PORT`
# automatically (see `crates/pellucid-edge-bin/src/config.rs`).

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

RUN useradd --uid 1000 --create-home --shell /usr/sbin/nologin pellucid \
 && mkdir -p /data \
 && chown pellucid:pellucid /data

COPY --from=builder /build/target/release/pellucid-edge-bin /usr/local/bin/pellucid-edge-bin
RUN chmod 755 /usr/local/bin/pellucid-edge-bin

USER pellucid
WORKDIR /data

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/pellucid-edge-bin"]
