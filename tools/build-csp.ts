#!/usr/bin/env bun
/**
 * tools/build-csp.ts — Single-source CSP builder (SPEC-001 §9, §25.2; M5 fix).
 *
 * Emits the canonical Content-Security-Policy header string for the
 * Pellucid webview. Three consumers must end up with byte-identical
 * policies (otherwise drift is silent):
 *   1. webview/index.html <meta http-equiv="Content-Security-Policy">
 *   2. crates/pellucid-edge-bin's set_csp Tower middleware (Rust constant)
 *   3. crates/pellucid-tauri/tauri.conf.json security.csp
 *
 * `tools/check-csp.ts` reads all three and verifies they match this
 * builder's output. The pre-push hook (T0.12) and CI gate run that check.
 *
 * Variant frame-src entries are derived from
 * webview/src/config/variants/*.ts so adding a new variant cannot drift
 * out of the CSP allowlist (L4 fix).
 */

import { readdirSync, statSync } from "node:fs";
import { join } from "node:path";
import { argv, exit } from "node:process";

export interface CspDirectives {
  "default-src": string[];
  "connect-src": string[];
  "script-src": string[];
  "style-src": string[];
  "img-src": string[];
  "font-src": string[];
  "frame-src": string[];
  "worker-src": string[];
  "object-src": string[];
  "base-uri": string[];
  "form-action": string[];
}

export const STATIC_HOSTS = {
  api: "https://api.worldmonitor.app",
  apex: "https://worldmonitor.app",
  clerk: "https://*.clerk.accounts.dev",
  convex: "https://*.convex.cloud",
  aisstream: "wss://stream.aisstream.io",
  tauri: "tauri://localhost",
  sidecar: "http://127.0.0.1:*",
  sentry: "https://*.sentry.io",
  dodo: "https://checkout.dodopayments.com",
} as const;

/** Discover variant subdomains by listing webview/src/config/variants. */
export function variantHosts(repoRoot: string): string[] {
  const dir = join(repoRoot, "webview/src/config/variants");
  let names: string[] = [];
  try {
    names = readdirSync(dir);
  } catch {
    return [];
  }
  const variants: string[] = [];
  for (const name of names) {
    const full = join(dir, name);
    if (!statSync(full).isFile()) continue;
    const slug = name.replace(/\.ts$/, "");
    if (slug === "index" || slug === "base") continue;
    variants.push(`https://${slug}.worldmonitor.app`);
  }
  return variants.sort();
}

export function buildDirectives(repoRoot: string): CspDirectives {
  const variants = variantHosts(repoRoot);
  return {
    "default-src": ["'self'"],
    "connect-src": [
      "'self'",
      STATIC_HOSTS.api,
      STATIC_HOSTS.apex,
      STATIC_HOSTS.clerk,
      STATIC_HOSTS.convex,
      STATIC_HOSTS.aisstream,
      STATIC_HOSTS.tauri,
      STATIC_HOSTS.sidecar,
      STATIC_HOSTS.sentry,
      STATIC_HOSTS.dodo,
    ],
    "script-src": ["'self'", "'wasm-unsafe-eval'", STATIC_HOSTS.clerk],
    "style-src": ["'self'", "'unsafe-inline'"],
    "img-src": ["'self'", "data:", "https:"],
    "font-src": ["'self'", "data:"],
    "frame-src": ["'self'", STATIC_HOSTS.dodo, ...variants],
    "worker-src": ["'self'", "blob:"],
    "object-src": ["'none'"],
    "base-uri": ["'self'"],
    "form-action": ["'self'", STATIC_HOSTS.dodo],
  };
}

export function serializeCsp(directives: CspDirectives): string {
  const parts: string[] = [];
  for (const [key, values] of Object.entries(directives)) {
    if (values.length === 0) continue;
    parts.push(`${key} ${values.join(" ")}`);
  }
  return parts.join("; ");
}

export function emit(repoRoot: string): string {
  return serializeCsp(buildDirectives(repoRoot));
}

async function main(): Promise<void> {
  const repoRoot = argv[2] ?? process.cwd();
  process.stdout.write(emit(repoRoot) + "\n");
  exit(0);
}

if (import.meta.main) {
  await main();
}
