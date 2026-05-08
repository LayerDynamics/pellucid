/**
 * WebDriverIO configuration for desktop end-to-end tests against the
 * built `pellucid-tauri` binary, brokered by `tauri-driver`.
 *
 * Why webdriverio rather than Playwright for the desktop suite:
 *   tauri-driver speaks the WebDriver protocol; webdriverio is the
 *   officially supported client. Playwright speaks Chrome DevTools
 *   Protocol and does not have a maintained WebDriver bridge for the
 *   WebKit/WebView engines Tauri uses on macOS and Linux. The web SPA
 *   continues to be tested under Playwright (see playwright.config.ts).
 *
 * Platform notes:
 *   - Linux CI runners must have `webkit2gtk-driver` installed.
 *   - Windows CI runners must have `msedgedriver` installed.
 *   - macOS support in tauri-driver is experimental as of 2026-Q1; the
 *     Justfile `e2e-desktop` target skips with a clear message when run
 *     on darwin until upstream support stabilizes.
 */

import path from "node:path";
import process from "node:process";

const repoRoot = path.dirname(new URL(import.meta.url).pathname);
const tauriBinary =
  process.env.PELLUCID_TAURI_BIN ??
  path.join(repoRoot, "target/release/pellucid-tauri");

export const config = {
  runner: "local",
  specs: ["./e2e/desktop/**/*.spec.ts"],
  maxInstances: 1,
  // tauri-driver speaks classic WebDriver with the `wry` browserName
  // (the webview engine Tauri embeds — `tauri` is not a recognised
  // value and crashes the session-create handshake with
  // "Failed to match capabilities").
  //
  // wdio v9 unconditionally injects `webSocketUrl: true` into the
  // outgoing capabilities (see webdriver/build/node.js:1014) UNLESS
  // `wdio:enforceWebDriverClassic: true` is also present —
  // `webSocketUrl: false` is silently ignored, so we use the documented
  // opt-out flag instead. tauri-driver only speaks the HTTP-W3C subset
  // and rejects the BiDi capability with "Failed to match capabilities".
  capabilities: [
    {
      browserName: "wry",
      "wdio:enforceWebDriverClassic": true,
      "tauri:options": {
        application: tauriBinary,
      },
    },
  ],
  hostname: "127.0.0.1",
  port: 4444,
  path: "/",
  logLevel: "info",
  bail: 0,
  baseUrl: "tauri://localhost",
  waitforTimeout: 10_000,
  connectionRetryTimeout: 30_000,
  connectionRetryCount: 3,
  framework: "mocha",
  reporters: ["spec"],
  mochaOpts: {
    ui: "bdd",
    timeout: 60_000,
  },
} satisfies import("@wdio/types").Options.Testrunner;

export default config;
