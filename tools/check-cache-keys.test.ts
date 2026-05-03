/**
 * Unit + integration tests for tools/check-cache-keys.ts.
 *
 * The lint operates on Rust source; we use checked-in fixture .rs files
 * (NOT the pellucid-handlers crate, which is empty until T2.5) so the
 * harness exercises both pass and fail paths from day one.
 */

import { describe, expect, test } from "bun:test";
import { join } from "node:path";

import { lintDirectory } from "./check-cache-keys";

const repoRoot = join(import.meta.dir, "..");

describe("check-cache-keys (unit)", () => {
  test("good fixtures produce zero findings", () => {
    const report = lintDirectory(join(repoRoot, "tools/test-fixtures/handlers-good"));
    expect(report.findings).toEqual([]);
    expect(report.filesScanned).toBe(2);
    expect(report.callsFound).toBe(2);
  });

  test("bad fixture flags missing field with line + key + field name", () => {
    const report = lintDirectory(join(repoRoot, "tools/test-fixtures/handlers-bad"));
    expect(report.findings.length).toBe(1);
    const finding = report.findings[0]!;
    expect(finding.file).toContain("missing_field.rs");
    expect(finding.cacheKey).toContain("aviation:status:fixed:v1");
    expect(finding.missingFields).toContain("flight");
    expect(finding.line).toBeGreaterThan(0);
  });

  test("missing directory yields zero findings without throwing", () => {
    const report = lintDirectory(join(repoRoot, "tools/test-fixtures/does-not-exist"));
    expect(report.filesScanned).toBe(0);
    expect(report.findings).toEqual([]);
  });

  test("turbofish callee is recognised + missing field is flagged", () => {
    // T2.10 fix: callees like `cached_fetch_json::<T, _, _>` were
    // invisible to the original endsWith match — the post-T2.5
    // production handler uses turbofish exclusively.
    const report = lintDirectory(
      join(repoRoot, "tools/test-fixtures/handlers-turbofish"),
    );
    expect(report.callsFound).toBe(1);
    expect(report.findings.length).toBe(1);
    const finding = report.findings[0]!;
    expect(finding.cacheKey).toContain("NO_FIELD");
    expect(finding.missingFields).toContain("flight");
  });

  test("trusted req.cache_key() method short-circuits the field check", () => {
    // T2.10 fix: when the cache-key argument is a method call
    // on a request receiver (`q.cache_key()` / `&key` from
    // such a binding), the linter trusts the typed method as
    // canonical. The fixture references `q.flight` + `q.date`;
    // the inline string `aviation:status:{}:{}:v1` (a format
    // template, not a placeholder pattern) doesn't include
    // either as `{flight}`/`{date}`, but the trust path skips
    // the comparison.
    const report = lintDirectory(
      join(repoRoot, "tools/test-fixtures/handlers-trusted"),
    );
    expect(report.callsFound).toBe(1);
    expect(report.findings).toEqual([]);
  });
});

describe("check-cache-keys (integration)", () => {
  test("real pellucid-handlers/src lints clean", () => {
    // The aviation handler (T2.5) + bootstrap handler (T2.7)
    // both call `cached_fetch_json` with a `&req.cache_key()`
    // / `&key`-from-method binding — the trust path should
    // make both pass.
    const report = lintDirectory(join(repoRoot, "crates/pellucid-handlers/src"));
    expect(report.findings).toEqual([]);
    // Sanity: the linter actually FOUND the calls (was 0
    // before turbofish detection was added).
    expect(report.callsFound).toBeGreaterThanOrEqual(1);
  });
});
