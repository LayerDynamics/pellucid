# Pellucid

Real-time situational-awareness console — markets, geopolitics, climate, energy, supply chain, infrastructure, cyber, intelligence — surfaced through a deck.gl 2D map and a globe.gl 3D map, with cross-source correlation and on-device ML.

Ships as a **Tauri** desktop app (offline-capable, locally cached) and a hosted SaaS (`worldmonitor.app` web SPA + `api.worldmonitor.app` public RPC API). Five domain-skinned variants: `base`, `tech`, `finance`, `commodity`, `happy`.

## Stack

| Layer | Tool |
|---|---|
| Desktop shell | Tauri 2 |
| UI runtime | React 19 + Vite 6 + TypeScript |
| Styling | Tailwind CSS 4 |
| UI primitives | Radix Primitives + Radix Colors |
| State | Zustand 5 |
| JS runtime / package manager / test runner | Bun 1.x |
| Local + edge canonical store | SQLite 3.46+ (FTS5, R*Tree, sqlite-vec, sqlite-zstd) |
| Non-public-facing systems | Rust (Tokio, Axum, Reqwest, SQLx, ort/candle) |
| Auth / payments / billing | Clerk + Dodo + Convex |

## Documentation

- **Spec**: [`docs/specs/SPEC-001-pellucid-stack-rebuild.md`](docs/specs/SPEC-001-pellucid-stack-rebuild.md)
- **Implementation plan**: [`docs/plans/2026-04-25-pellucid-rebuild.md`](docs/plans/2026-04-25-pellucid-rebuild.md)
- **Source review** (parity reference): [`docs/LoreDeepCodeReview.md`](docs/LoreDeepCodeReview.md), [`docs/LoreWorldMonitorComponents.md`](docs/LoreWorldMonitorComponents.md)

## Repository layout

```text
crates/                # Rust workspace (15 crates per spec §11)
webview/               # Vite + React + Tailwind + Radix + Zustand bundle
convex/                # Convex schema + billing/webhook actions
proto/                 # buf + sebuf RPC definitions
tools/                 # Bun-driven dev/CI scripts
docs/                  # specs, plans, ADRs, source review
deploy/                # Fly.io manifests
docker/                # Container definitions
e2e/                   # Playwright specs (web + desktop)
.github/workflows/     # CI
```

## Getting started

```bash
just install           # bun install + cargo fetch
just check             # full CI gate locally
just dev-desktop       # cargo tauri dev
just dev-web           # bun run --filter=webview dev
```

## License

MIT — see [LICENSE](LICENSE).
