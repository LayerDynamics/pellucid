# Pellucid end-to-end tests

Two suites, two runners:

| Suite | Runner | Target | Trigger |
|---|---|---|---|
| `e2e/*.spec.ts` (excl. `desktop/`, `visual/`) | Playwright `web` project | Vite dev server (`bun run dev`) | `bunx playwright test` |
| `e2e/visual/*.spec.ts` | Playwright `web-visual` project | Vite dev server | `bunx playwright test --project=web-visual` |
| `e2e/desktop/*.spec.ts` | WebDriverIO via `tauri-driver` | Bundled `pellucid-tauri` release binary | `bun run e2e:desktop` |

## Why two runners

`tauri-driver` speaks the WebDriver protocol; WebDriverIO is the officially
supported client. Playwright cannot drive the WebKit/WebView engines Tauri
uses on macOS or the `webkit2gtk` engine on Linux, so the desktop suite
runs under WebDriverIO while the hosted web SPA stays on Playwright.

## Platform support for desktop e2e

| OS | tauri-driver backend | Status |
|---|---|---|
| Linux | `webkit2gtk-driver` (apt: `webkit2gtk-driver`) | Supported in CI |
| Windows | `msedgedriver` | Supported in CI |
| macOS | experimental | Skipped locally; revisit when upstream stabilizes |

The Justfile's `e2e-desktop` target detects the host OS and skips on macOS
with a clear message rather than failing.

## Cassettes (Polly.js)

Tests that exercise upstream HTTP (e.g. T2.5+ aviation handler integration)
record cassettes via `@pollyjs/persister-fs` into `e2e/cassettes/<test-id>/`.
Cassettes are committed; CI runs in replay mode (`POLLY_MODE=replay`).
