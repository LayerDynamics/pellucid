/**
 * Unit tests for tools/version-sync.ts.
 */

import { describe, expect, test } from "bun:test";
import { join } from "node:path";

import { findDrift, readJsonVersion, readWorkspaceVersion } from "./version-sync";

const repoRoot = join(import.meta.dir, "..");

describe("version-sync (unit)", () => {
  test("readWorkspaceVersion returns the [workspace.package].version", () => {
    const version = readWorkspaceVersion(repoRoot);
    expect(version).toMatch(/^\d+\.\d+\.\d+/);
  });

  test("readJsonVersion handles missing files gracefully", () => {
    const src = readJsonVersion(repoRoot, "does-not-exist.json");
    expect(src.version).toBeNull();
    expect(src.path).toBe("does-not-exist.json");
  });

  test("readJsonVersion reads a real package.json", () => {
    const src = readJsonVersion(repoRoot, "package.json");
    expect(src.version).toMatch(/^\d+\.\d+\.\d+/);
  });

  test("findDrift returns the workspace version and zero drift in a clean repo", () => {
    const { expected, drifts } = findDrift(repoRoot);
    expect(expected).toMatch(/^\d+\.\d+\.\d+/);
    expect(drifts).toEqual([]);
  });
});
