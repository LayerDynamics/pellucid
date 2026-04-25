#!/usr/bin/env bun
/**
 * tools/check-csp.ts — Verifies CSP triplication parity (SPEC-001 §9, §25.2; M5 fix).
 *
 * Reads the CSP from the three locations:
 *   1. webview/index.html  <meta http-equiv="Content-Security-Policy" content="...">
 *   2. crates/pellucid-edge-bin/src/middleware.rs  PELLUCID_CSP constant
 *   3. crates/pellucid-tauri/tauri.conf.json  security.csp
 *
 * Compares each against the canonical output of build-csp.ts. Exits
 * non-zero on any divergence with a clear diff per location.
 *
 * Locations not yet wired (e.g. tauri.conf.json security.csp is `null`
 * at T0.6, the edge middleware lands at T2.6) are reported as "absent"
 * but do not fail the check until T6.3 — they only fail if the file
 * exists AND the content disagrees.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { exit } from "node:process";

import { emit } from "./build-csp";

export type Source = "index.html" | "edge-middleware" | "tauri.conf.json";

export interface ExtractedCsp {
  source: Source;
  status: "match" | "mismatch" | "absent";
  found?: string;
}

const META_RE = /<meta[^>]*http-equiv=["']Content-Security-Policy["'][^>]*content=["']([^"']+)["']/i;
const RUST_CONST_RE = /pub(?:\(crate\))?\s+const\s+PELLUCID_CSP\s*:\s*&str\s*=\s*"([^"]+)";/;

export function readIndexHtmlCsp(repoRoot: string): string | null {
  const path = join(repoRoot, "webview/index.html");
  let html: string;
  try {
    html = readFileSync(path, "utf8");
  } catch {
    return null;
  }
  const m = META_RE.exec(html);
  return m ? m[1]! : null;
}

export function readEdgeMiddlewareCsp(repoRoot: string): string | null {
  const path = join(repoRoot, "crates/pellucid-edge-bin/src/middleware.rs");
  let src: string;
  try {
    src = readFileSync(path, "utf8");
  } catch {
    return null;
  }
  const m = RUST_CONST_RE.exec(src);
  return m ? m[1]! : null;
}

export function readTauriCsp(repoRoot: string): string | null {
  const path = join(repoRoot, "crates/pellucid-tauri/tauri.conf.json");
  let raw: string;
  try {
    raw = readFileSync(path, "utf8");
  } catch {
    return null;
  }
  const cfg = JSON.parse(raw) as { app?: { security?: { csp?: string | null } } };
  const csp = cfg.app?.security?.csp;
  return typeof csp === "string" && csp.length > 0 ? csp : null;
}

export function diffCsp(expected: string, actual: string): { ok: boolean; diff: string } {
  if (expected === actual) return { ok: true, diff: "" };
  const expectedDirectives = expected.split(/\s*;\s*/).filter(Boolean).sort();
  const actualDirectives = actual.split(/\s*;\s*/).filter(Boolean).sort();
  const expectedSet = new Set(expectedDirectives);
  const actualSet = new Set(actualDirectives);
  const missing = expectedDirectives.filter((d) => !actualSet.has(d));
  const extra = actualDirectives.filter((d) => !expectedSet.has(d));
  const lines: string[] = [];
  for (const m of missing) lines.push(`  - missing: ${m}`);
  for (const e of extra) lines.push(`  - extra:   ${e}`);
  return { ok: false, diff: lines.join("\n") };
}

export function check(repoRoot: string): ExtractedCsp[] {
  const expected = emit(repoRoot);
  const html = readIndexHtmlCsp(repoRoot);
  const edge = readEdgeMiddlewareCsp(repoRoot);
  const tauri = readTauriCsp(repoRoot);

  const out: ExtractedCsp[] = [];
  for (const [source, found] of [
    ["index.html", html],
    ["edge-middleware", edge],
    ["tauri.conf.json", tauri],
  ] as Array<[Source, string | null]>) {
    if (found === null) {
      out.push({ source, status: "absent" });
      continue;
    }
    out.push({
      source,
      status: found === expected ? "match" : "mismatch",
      found,
    });
  }
  return out;
}

async function main(): Promise<void> {
  const repoRoot = process.cwd();
  const expected = emit(repoRoot);
  const results = check(repoRoot);

  let mismatched = 0;
  for (const r of results) {
    if (r.status === "absent") {
      process.stdout.write(`[check-csp] ${r.source}: absent (will be wired by a later task)\n`);
      continue;
    }
    if (r.status === "match") {
      process.stdout.write(`[check-csp] ${r.source}: ok\n`);
      continue;
    }
    const { diff } = diffCsp(expected, r.found!);
    process.stderr.write(`[check-csp] ${r.source}: MISMATCH\n${diff}\n`);
    mismatched += 1;
  }

  if (mismatched > 0) {
    process.stderr.write(
      `[check-csp] ${mismatched} location(s) diverge; run \`bun run tools/build-csp.ts\` and update.\n`,
    );
    exit(1);
  }
  exit(0);
}

if (import.meta.main) {
  await main();
}
