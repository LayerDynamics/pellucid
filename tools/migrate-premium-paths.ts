#!/usr/bin/env bun
/**
 * tools/migrate-premium-paths.ts — H4 FIX migration driver.
 *
 * Reads `data/premium-rpc-paths.json` (the 33-entry reference list
 * for the legacy `PREMIUM_RPC_PATHS` Bearer-`role='pro'` paths from
 * the original WorldMonitor `server/gateway.ts:312-358`), folds it
 * into the existing `crates/pellucid-auth/src/endpoint_tiers.rs`
 * `ENDPOINT_ENTITLEMENTS` table, and writes the result back —
 * preserving the 4 pre-existing tier-2 entries.
 *
 * Per SPEC-001 §14.4 the migration produces a strict superset on
 * day one (37 entries = 4 tier-2 + 33 tier-1), and the gateway
 * carries no separate `PREMIUM_RPC_PATHS` code path. The
 * regression test at `crates/pellucid-gateway/tests/regression_h4.rs`
 * verifies the gating works end-to-end for every entry; the
 * `committed_table_matches_reference_data` test in
 * `endpoint_tiers.rs` itself locks the live table to this script's
 * output so no manual edit can drift it.
 *
 * Usage:
 *   bun run tools/migrate-premium-paths.ts                 # check
 *   bun run tools/migrate-premium-paths.ts --regenerate    # rewrite
 *   bun run tools/migrate-premium-paths.ts --emit-rust     # stdout
 *   bun run tools/migrate-premium-paths.ts --emit-json     # stdout
 *
 * Exit codes:
 *   0 — table matches reference data (or --regenerate succeeded)
 *   1 — table differs from reference data
 *   2 — IO / parse / validation error
 */

import { readFileSync, writeFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..");
const REFERENCE_FILE = resolve(REPO, "data/premium-rpc-paths.json");
const ENDPOINT_TIERS_FILE = resolve(
  REPO,
  "crates/pellucid-auth/src/endpoint_tiers.rs",
);

/** Reference-list entry as serialised in `data/premium-rpc-paths.json`. */
export interface ReferenceEntry {
  /** `/api/{domain}/v1/{rpc}` */
  path: string;
  /** Numeric `Tier::rank()`. */
  tier: number;
  /** Domain — purely diagnostic, kept so a `jq '.paths[] | .domain'`
      sweep is possible. */
  domain?: string;
}

/** Strict shape we emit into the Rust source. Sorted by `path`. */
export interface RustEntry {
  path: string;
  tier: number;
}

/** The 4 pre-existing tier-2 entries that were in the table before
 *  the H4 migration. The migration script preserves these verbatim
 *  — they are the canonical SPEC-001 §14.2 set and exist in the
 *  reference data only as already-tier-2 entries (they are NOT in
 *  premium-rpc-paths.json which holds only the *legacy* set). */
export const PRE_EXISTING_TIER2: readonly RustEntry[] = [
  { path: "/api/market/v1/analyze-stock", tier: 3 },
  { path: "/api/market/v1/get-stock-analysis-history", tier: 3 },
  { path: "/api/market/v1/backtest-stock", tier: 3 },
  { path: "/api/market/v1/list-stored-stock-backtests", tier: 3 },
] as const;

/** Read + minimal validation of the reference data file. */
export function loadReference(path = REFERENCE_FILE): ReferenceEntry[] {
  const raw = readFileSync(path, "utf8");
  const parsed = JSON.parse(raw) as { paths?: ReferenceEntry[] };
  if (!parsed || !Array.isArray(parsed.paths)) {
    throw new Error(`${path}: missing top-level "paths" array`);
  }
  for (const e of parsed.paths) {
    if (typeof e.path !== "string" || !e.path.startsWith("/api/")) {
      throw new Error(`${path}: bad entry ${JSON.stringify(e)}`);
    }
    if (
      typeof e.tier !== "number" ||
      !Number.isInteger(e.tier) ||
      e.tier < 0 ||
      e.tier > 3
    ) {
      throw new Error(
        `${path}: ${e.path} has out-of-range tier ${String(e.tier)}`,
      );
    }
  }
  // Reject duplicates inside the reference data itself.
  const seen = new Set<string>();
  for (const e of parsed.paths) {
    if (seen.has(e.path)) {
      throw new Error(`${path}: duplicate path ${e.path}`);
    }
    seen.add(e.path);
  }
  return parsed.paths;
}

/** Combine pre-existing tier-2 entries with the migrated reference
 *  set. Sorts deterministically by path so the emitted Rust source
 *  is stable across runs. */
export function buildSuperset(
  reference: readonly ReferenceEntry[],
  preExisting: readonly RustEntry[] = PRE_EXISTING_TIER2,
): RustEntry[] {
  const all: RustEntry[] = [
    ...preExisting.map((e) => ({ path: e.path, tier: e.tier })),
    ...reference.map((e) => ({ path: e.path, tier: e.tier })),
  ];
  // Reject collisions between pre-existing and reference sets.
  const seen = new Map<string, number>();
  for (const e of all) {
    const prev = seen.get(e.path);
    if (prev !== undefined && prev !== e.tier) {
      throw new Error(
        `path ${e.path} appears in both pre-existing (tier ${prev}) and reference (tier ${e.tier})`,
      );
    }
    seen.set(e.path, e.tier);
  }
  // De-dupe (a path appearing twice with the same tier collapses).
  const dedup = new Map<string, RustEntry>();
  for (const e of all) {
    dedup.set(e.path, e);
  }
  return [...dedup.values()].sort((a, b) =>
    a.path < b.path ? -1 : a.path > b.path ? 1 : 0,
  );
}

/** Produce the literal `&[(&str, u8)]` Rust source for the
 *  `ENDPOINT_ENTITLEMENTS` table. The output is what the existing
 *  `endpoint_tiers.rs` already uses (linear scan, drop the `phf`
 *  dep — see refactor commit `cb66e93`). */
export function emitRustTable(entries: readonly RustEntry[]): string {
  const longest = Math.max(...entries.map((e) => e.path.length));
  const lines = entries.map(
    (e) => `    (${quote(e.path)},${" ".repeat(longest - e.path.length + 1)}${e.tier}),`,
  );
  return [
    "pub const ENDPOINT_ENTITLEMENTS: &[(&str, u8)] = &[",
    ...lines,
    "];",
  ].join("\n");
}

function quote(s: string): string {
  return `"${s}"`;
}

/** Read the live `endpoint_tiers.rs` and return the table block
 *  bracketed by `pub const ENDPOINT_ENTITLEMENTS: &[(&str, u8)] = &[`
 *  and the closing `];`. Includes the brackets. */
export function extractTableBlock(source: string): {
  start: number;
  end: number;
  text: string;
} {
  const startMarker = "pub const ENDPOINT_ENTITLEMENTS: &[(&str, u8)] = &[";
  const start = source.indexOf(startMarker);
  if (start === -1) {
    throw new Error("ENDPOINT_ENTITLEMENTS table not found");
  }
  // Find the matching `];` after `start`.
  const endMarker = "];";
  const end = source.indexOf(endMarker, start);
  if (end === -1) {
    throw new Error("ENDPOINT_ENTITLEMENTS table is unterminated");
  }
  return {
    start,
    end: end + endMarker.length,
    text: source.slice(start, end + endMarker.length),
  };
}

/** Splice a freshly-rendered table back into the file body. */
export function spliceTable(source: string, table: string): string {
  const { start, end } = extractTableBlock(source);
  return source.slice(0, start) + table + source.slice(end);
}

interface CliOptions {
  regenerate: boolean;
  emitRust: boolean;
  emitJson: boolean;
}

function parseArgs(argv: readonly string[]): CliOptions {
  const opts: CliOptions = {
    regenerate: false,
    emitRust: false,
    emitJson: false,
  };
  for (const a of argv) {
    if (a === "--regenerate") opts.regenerate = true;
    else if (a === "--emit-rust") opts.emitRust = true;
    else if (a === "--emit-json") opts.emitJson = true;
    else if (a === "-h" || a === "--help") {
      console.log(
        "usage: bun run tools/migrate-premium-paths.ts " +
          "[--regenerate] [--emit-rust] [--emit-json]",
      );
      process.exit(0);
    } else {
      console.error(`unknown flag: ${a}`);
      process.exit(2);
    }
  }
  return opts;
}

function main(): void {
  const opts = parseArgs(process.argv.slice(2));
  let entries: RustEntry[];
  try {
    entries = buildSuperset(loadReference());
  } catch (e) {
    console.error(`error: ${(e as Error).message}`);
    process.exit(2);
  }
  const table = emitRustTable(entries);

  if (opts.emitJson) {
    console.log(JSON.stringify({ entries }, null, 2));
    return;
  }
  if (opts.emitRust) {
    console.log(table);
    return;
  }

  const source = readFileSync(ENDPOINT_TIERS_FILE, "utf8");
  const { text: live } = extractTableBlock(source);
  if (live === table) {
    console.log(`ok: endpoint_tiers.rs matches reference (${entries.length} entries)`);
    return;
  }
  if (opts.regenerate) {
    writeFileSync(ENDPOINT_TIERS_FILE, spliceTable(source, table));
    console.log(`regenerated endpoint_tiers.rs (${entries.length} entries)`);
    return;
  }
  console.error("ERROR: endpoint_tiers.rs is out of date.\n");
  console.error("Live table block:\n");
  console.error(live);
  console.error("\nExpected from reference data:\n");
  console.error(table);
  console.error(
    "\nRun: bun run tools/migrate-premium-paths.ts --regenerate",
  );
  process.exit(1);
}

if (import.meta.main) {
  main();
}
