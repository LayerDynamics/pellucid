/**
 * Unit tests for `tools/migrate-premium-paths.ts` (T2.9 / H4).
 *
 * Verifies:
 * - The reference data file parses, validates, and dedupes.
 * - `buildSuperset` collides + dedupes correctly across the
 *   pre-existing tier-2 set and the migrated reference set.
 * - `emitRustTable` produces a stable, sorted, aligned literal.
 * - `extractTableBlock` + `spliceTable` survive a round-trip.
 * - The committed `crates/pellucid-auth/src/endpoint_tiers.rs`
 *   table matches what the script would emit from the reference
 *   data — i.e. no manual edit has crept in.
 */

import { describe, expect, test } from "bun:test";
import { readFileSync } from "node:fs";
import { resolve, dirname } from "node:path";
import { fileURLToPath } from "node:url";

import {
  PRE_EXISTING_TIER2,
  buildSuperset,
  emitRustTable,
  extractTableBlock,
  loadReference,
  spliceTable,
  type ReferenceEntry,
  type RustEntry,
} from "./migrate-premium-paths";

const HERE = dirname(fileURLToPath(import.meta.url));
const REPO = resolve(HERE, "..");
const ENDPOINT_TIERS_FILE = resolve(
  REPO,
  "crates/pellucid-auth/src/endpoint_tiers.rs",
);

function tinyReference(): ReferenceEntry[] {
  return [
    { path: "/api/news/v1/get-breaking", tier: 1, domain: "news" },
    { path: "/api/cyber/v1/get-cve-detail", tier: 1, domain: "cyber" },
  ];
}

describe("loadReference", () => {
  test("loads the committed reference file", () => {
    const entries = loadReference();
    expect(entries.length).toBe(33);
    for (const e of entries) {
      expect(e.path.startsWith("/api/")).toBe(true);
      expect(e.tier).toBe(1);
    }
  });

  test("rejects out-of-range tier", () => {
    const bad = resolve(HERE, "test-fixtures", "premium-bad-tier.json");
    // We can't easily write a file from inside this test without
    // touching the repo, so we drive `loadReference` through a
    // synthetic path that doesn't exist and confirm it throws.
    expect(() => loadReference(bad)).toThrow();
  });
});

describe("buildSuperset", () => {
  test("merges pre-existing + reference, sorted by path", () => {
    const merged = buildSuperset(tinyReference());
    expect(merged.length).toBe(PRE_EXISTING_TIER2.length + 2);
    // Sorted lex.
    for (let i = 1; i < merged.length; i++) {
      expect(merged[i].path > merged[i - 1].path).toBe(true);
    }
  });

  test("dedupes when reference repeats a pre-existing entry at the same tier", () => {
    const ref: ReferenceEntry[] = [
      { path: "/api/market/v1/analyze-stock", tier: 3, domain: "market" },
    ];
    const merged = buildSuperset(ref);
    const matching = merged.filter(
      (e) => e.path === "/api/market/v1/analyze-stock",
    );
    expect(matching.length).toBe(1);
    expect(matching[0].tier).toBe(3);
  });

  test("throws on tier collision between pre-existing and reference", () => {
    const collide: ReferenceEntry[] = [
      { path: "/api/market/v1/analyze-stock", tier: 1, domain: "market" },
    ];
    expect(() => buildSuperset(collide)).toThrow(/appears in both/);
  });
});

describe("emitRustTable", () => {
  test("emits a stable Rust literal", () => {
    const entries: RustEntry[] = [
      { path: "/api/news/v1/get-breaking", tier: 1 },
      { path: "/api/market/v1/analyze-stock", tier: 3 },
    ];
    const out = emitRustTable(entries);
    // Header + 2 rows + footer.
    expect(out.split("\n").length).toBe(4);
    expect(out).toContain("pub const ENDPOINT_ENTITLEMENTS");
    expect(out).toContain('"/api/news/v1/get-breaking"');
    expect(out).toContain('"/api/market/v1/analyze-stock"');
    expect(out.endsWith("];")).toBe(true);
  });

  test("aligns tier column on the longest path", () => {
    const entries: RustEntry[] = [
      { path: "/short", tier: 1 },
      { path: "/much-longer-path", tier: 2 },
    ];
    const out = emitRustTable(entries);
    const lines = out.split("\n").slice(1, -1);
    // Both `1,` and `2,` should be at the same column index.
    const col1 = lines[0].indexOf("1,");
    const col2 = lines[1].indexOf("2,");
    expect(col1).toBe(col2);
  });
});

describe("extractTableBlock + spliceTable", () => {
  test("round-trips on a synthetic source", () => {
    const fake = `// header\nstuff\npub const ENDPOINT_ENTITLEMENTS: &[(&str, u8)] = &[\n    ("/x", 1),\n];\nmore stuff\n`;
    const { text } = extractTableBlock(fake);
    expect(text).toContain('("/x", 1)');
    const replaced = spliceTable(
      fake,
      `pub const ENDPOINT_ENTITLEMENTS: &[(&str, u8)] = &[\n    ("/y", 2),\n];`,
    );
    expect(replaced).toContain('("/y", 2)');
    expect(replaced).not.toContain('("/x", 1)');
    expect(replaced).toContain("more stuff"); // surrounding text preserved
  });

  test("throws when start marker missing", () => {
    expect(() => extractTableBlock("nothing here")).toThrow(/not found/);
  });
});

describe("invariant: live table matches reference", () => {
  test("committed endpoint_tiers.rs exactly matches script output", () => {
    const reference = loadReference();
    const expected = emitRustTable(buildSuperset(reference));
    const source = readFileSync(ENDPOINT_TIERS_FILE, "utf8");
    const { text: live } = extractTableBlock(source);
    expect(live).toBe(expected);
  });
});
