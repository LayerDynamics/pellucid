# Railway deployment runbook

Pellucid ships two Railway services — `pellucid-edge` and
`pellucid-relay` — backed by the Dockerfiles in `edge/` and
`relay/`. Per `docs/specs/SPEC-001-pellucid-stack-rebuild.md`
§5.2 every other Pellucid crate is a library linked into one of
these two binaries; there are no other Railway services.

This document is the operational runbook: env vars, multi-region
setup, custom-domain wiring, Litestream/SQLite replication, and
post-deploy checks. The CI side (image build, dry-run validate,
post-deploy smoke probe) lives in `.github/workflows/deploy.yml`
and `.github/workflows/docker-publish.yml`.

## Service inventory

| Service       | Crate                | Dockerfile                          | Hostname today           | Spec target                                                                  |
| ------------- | -------------------- | ----------------------------------- | ------------------------ | ---------------------------------------------------------------------------- |
| `pellucid-edge`  | `pellucid-edge-bin`  | `deploy/railway/edge/Dockerfile`    | `pellucid.world`         | `worldmonitor.app` apex + 4 variant subdomains + `api.worldmonitor.app`     |
| `pellucid-relay` | `pellucid-relay-bin` | `deploy/railway/relay/Dockerfile`   | `pellucid.wtf`           | one Railway service, single region (closest to upstream APIs) + failover    |

The edge binary serves both the public RPC API AND the SaaS web
SPA from the same Axum process — see SPEC-001 §17 and
`crates/pellucid-edge-bin/src/lib.rs:135` (`fallback_service` that
mounts `tower_http::services::ServeDir` for the bundled
`webview/dist/`).

## Required env vars (edge)

Set these in the Railway dashboard for the `pellucid-edge`
service before the first deploy. Anything marked **secret** must
NOT be committed to the repo.

### Application — auth + entitlements

| Var                       | Required | Source              | Notes                                                                                              |
| ------------------------- | :------: | ------------------- | -------------------------------------------------------------------------------------------------- |
| `CLERK_JWT_ISSUER`        | yes      | Clerk dashboard     | JWKS issuer URL, e.g. `https://<tenant>.clerk.accounts.dev`                                       |
| `CLERK_JWT_AUDIENCE`      | yes      | Clerk dashboard     | Expected `aud` claim (Clerk's "API audience" value)                                               |
| `RELAY_SHARED_SECRET`     | yes      | secret              | Must equal the `pellucid-relay` service's value of the same name                                  |
| `AVIATIONSTACK_API_KEY`   | yes      | aviationstack.com   | Plan determines per-handler rate limit; free tier is sufficient for solo dev                      |
| `CONVEX_DEPLOY_KEY`       | yes      | Convex dashboard    | Read key for entitlement-cache fallback                                                            |

### Application — SaaS routing

| Var                          | Required | Default                   | Notes                                                                                                                                       |
| ---------------------------- | :------: | ------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `PELLUCID_API_HOST_PREFIX`   | optional | unset                     | Set to `api.` if you split `api.<apex>` from the apex/variant hosts. Requests with `Host: api.<anything>` get a 404 for non-API paths instead of the SPA. SPEC-001 §17. Leave unset for single-domain deploys (the current `pellucid.world` setup). |
| `PELLUCID_WEBVIEW_DIST`      | optional | `/srv/webview` if exists  | Filesystem path the SPA bundle is read from. The Dockerfile copies `webview/dist/` to `/srv/webview` automatically.                         |
| `PELLUCID_RELAY_BASE_URL`    | optional | (none — gateway 503s)     | Internal URL of the relay service. Use `http://${{relay.RAILWAY_PRIVATE_DOMAIN}}:3004` to wire over Railway's private IPv6 network.        |
| `AVIATIONSTACK_BASE_URL`     | optional | `https://api.aviationstack.com/v1` | Override for staging or wiremock'd tests                                                                                                    |
| `RUST_LOG`                   | optional | (set in `[deploy.envs]`)  | Tracing filter. Default is `info,pellucid_edge_bin=info,pellucid_gateway=info,pellucid_handlers=info`                                       |

### Storage — Litestream replica (Digital Ocean Spaces)

Continuous SQLite replication to **Digital Ocean Spaces** is the
project's chosen durable-storage backend (SPEC-001 §6 / §17.5
calls for an S3-compatible store; Spaces is the configured
provider). The config file lives at
`deploy/railway/<service>/litestream.yml` and reads everything
from env so **no secrets touch the repo**. Both services
(`pellucid-edge` and `pellucid-relay`) are configured with
Litestream sidecars and need the same env-var set on each
Railway service — only the bucket path differs to keep WAL
frames separate.

| Var                              | Required | Notes                                                                                                                                       |
| -------------------------------- | :------: | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `LITESTREAM_REPLICA_URL`         | yes\*    | Full S3-protocol URL with the Digital Ocean Spaces endpoint encoded as a query string. **Empty value disables Litestream entirely** (acceptable for staging only). |
|                                  |          | edge service: `s3://allbuckets-1778293858293/pellucid-edge?endpoint=https://atl1.digitaloceanspaces.com&region=atl1&force-path-style=true` |
|                                  |          | relay service: `s3://allbuckets-1778293858293/pellucid-relay?endpoint=https://atl1.digitaloceanspaces.com&region=atl1&force-path-style=true` |
|                                  |          | (Other S3-compatible backends — Cloudflare R2, AWS S3, Backblaze B2 — work via the same URL form by swapping the `endpoint=` host. The Litestream config does not hard-code a provider.) |
| `LITESTREAM_ACCESS_KEY_ID`       | yes\*    | Digital Ocean Spaces access key id. **secret**                                                                                             |
| `LITESTREAM_SECRET_ACCESS_KEY`   | yes\*    | Matching Digital Ocean Spaces secret key. **secret**                                                                                       |
| `LITESTREAM_SYNC_INTERVAL`       | optional | Default `1s` — frequency of WAL ship                                                                                                       |
| `LITESTREAM_RETENTION`           | optional | Default `24h` — WAL retention window                                                                                                       |
| `LITESTREAM_SNAPSHOT_INTERVAL`   | optional | Default `1h` — full-snapshot cadence                                                                                                       |

\* Required when Litestream is enabled (i.e. `LITESTREAM_REPLICA_URL` is non-empty).

### Volume

Attach a Railway Volume mounted at `/data`. The binary opens
SQLite at `/data/pellucid-edge.db` (default
`PELLUCID_DB_URL`); Litestream restores from the replica into
this path on cold boot, then continuously replicates back out.
The Volume is the local working set, NOT the durable copy —
durability lives in the S3 bucket.

### Port

Railway sets `PORT` automatically. The binary's
`resolve_listen_addr_from_env`
(`crates/pellucid-edge-bin/src/config.rs:102`) reads it and
binds `0.0.0.0:$PORT`. Leave `PELLUCID_LISTEN_ADDR` unset.

## Required env vars (relay)

See `deploy/railway/relay/railway.toml` for the canonical list.
Briefly: `RELAY_SHARED_SECRET` (must match edge), `OPENSKY_*`
oauth2 creds, `AIS_API_KEY`, optional Telegram MTProto creds
behind the `telegram` cargo feature.

## Multi-region setup

SPEC-001 §17 OD-2 (locked 2026-05-04) calls for edge in three
regions: us-east, eu-west, ap-southeast. Railway exposes region
selection via the dashboard, not config-as-code, so this is a
per-service operational step:

1. In the Railway dashboard, open the `pellucid-edge` service.
2. **Settings → Regions** — add `us-east`, `eu-west`,
   `ap-southeast`. Railway runs one replica of the same image in
   each region.
3. **Settings → Networking** — enable the public domain. Railway
   load-balances incoming traffic across all enabled regions.
4. Each region needs its own Volume; Litestream replicates
   independently from each region to the same S3 bucket using
   per-region prefixes set via `LITESTREAM_REPLICA_URL` (e.g.
   `s3://bucket/pellucid-edge/us-east?endpoint=...`).

The relay service stays single-region (closest to upstream
APIs — typically us-east for AIS/OpenSky/RSS).

## Custom-domain wiring

`pellucid.world` (apex) and `pellucid.wtf` (relay) are the
current production domains. To add SPEC-001's
`worldmonitor.app` apex + 4 variant subdomains + `api.<apex>`:

1. **Railway dashboard → Service → Settings → Public Networking
   → Add Domain** for each:
   - `worldmonitor.app` (apex)
   - `tech.worldmonitor.app`
   - `finance.worldmonitor.app`
   - `commodity.worldmonitor.app`
   - `happy.worldmonitor.app`
   - `api.worldmonitor.app`
2. Railway issues a CNAME / ALIAS target per domain. Point the
   DNS records there.
3. Set `PELLUCID_API_HOST_PREFIX=api.` in the edge service's env
   so requests with `Host: api.worldmonitor.app` get a 404 for
   non-API paths instead of the SPA.
4. Variant detection on the apex/variant subdomains is automatic
   — `webview/src/config/variant.ts:62` resolves variant from
   `window.location.hostname` at runtime. The same Vite bundle
   serves all five variant hosts.

## CI integration

`deploy.yml` runs three phases on every push to `main`:

1. **validate** — `cargo fmt`, `cargo clippy --workspace`, full
   `cargo nextest`. Bun build of `webview/dist/` runs first
   because tauri-codegen panics if the dist is missing (see
   `crates/pellucid-tauri/Cargo.toml` `custom-protocol` feature).
2. **build-images** — matrix dry-run of `edge` + `relay`
   Dockerfiles. Fails the workflow if either Dockerfile bitrots
   before Railway sees it.
3. **smoke-deployed** — polls `https://${HOST}/healthz` (edge)
   and `https://${HOST}/health` (relay) for up to 10 minutes
   after Railway's auto-deploy finishes, fails if either host
   doesn't return 200. The hosts are configured via repo
   variables `PELLUCID_EDGE_PUBLIC_HOST` and
   `PELLUCID_RELAY_PUBLIC_HOST`.

`docker-publish.yml` builds and pushes per-arch images for both
services to GHCR (`ghcr.io/layerdynamics/pellucid-{edge,relay}`)
on every push to `main`. Railway's GitHub integration triggers
the actual deploy directly from the repo; the GHCR push is a
mirror for off-Railway hosts and rollback targets.

## Post-deploy checks

```sh
# Liveness — hits the binary's /healthz route directly:
curl -sS -o /dev/null -w "%{http_code}\n" https://pellucid.world/healthz
# Expect: 200

# SaaS SPA — hits the SPA fallback so the React Router boots:
curl -sS https://pellucid.world/ | head -5
# Expect: <!doctype html><html...> with the Vite-built index.html

# Public RPC API — round-trips through the gateway middleware:
curl -sS -H "origin: https://pellucid.world" \
  "https://pellucid.world/api/aviation/v1/get-flight-status?flight=AA100&date=2026-04-25&origin=JFK"
# Expect: JSON envelope (200 OK), or 503 with Retry-After if AVIATIONSTACK_API_KEY misconfigured
```

If `Host: api.<apex>` is wired and `PELLUCID_API_HOST_PREFIX=api.`
is set:

```sh
# api.* host should NOT serve the SPA:
curl -sS -H "host: api.pellucid.world" -o /dev/null -w "%{http_code}\n" \
  https://pellucid.world/dashboard
# Expect: 404 (not the SPA shell)
```

## Litestream operations

Restore the database manually from the replica (e.g. for a
one-off staging spin-up):

```sh
litestream restore -config /etc/litestream/litestream.yml /data/pellucid-edge.db
```

List snapshots in the replica:

```sh
litestream snapshots -config /etc/litestream/litestream.yml /data/pellucid-edge.db
```

The entrypoint script (`deploy/railway/edge/entrypoint.sh`) does
the restore + replicate-and-exec dance automatically on every
container start, so manual ops are only for backfills /
disaster-recovery drills.
