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
});

describe("check-cache-keys (integration)", () => {
  test("real pellucid-handlers/src lints clean (empty until T2.5)", () => {
    const report = lintDirectory(join(repoRoot, "crates/pellucid-handlers/src"));
    expect(report.findings).toEqual([]);
  });
});
