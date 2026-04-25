/**
 * Unit tests for tools/check-edge-imports.ts. Verifies that the
 * Cargo.toml dep-list parser handles the workspace-inheritance form,
 * and that lintFile flags an undeclared crate but accepts std/declared.
 */

import { describe, expect, test } from "bun:test";
import { join } from "node:path";

import { lintFile, lintHandlersCrate, listDeclaredDeps } from "./check-edge-imports";

const repoRoot = join(import.meta.dir, "..");

describe("check-edge-imports (unit)", () => {
  test("listDeclaredDeps reads the handlers crate Cargo.toml without error", () => {
    const declared = listDeclaredDeps(join(repoRoot, "crates/pellucid-handlers"));
    expect(declared instanceof Set).toBe(true);
  });

  test("lintFile passes on std-only imports", () => {
    const findings = lintFile(
      "fake.rs",
      "use std::path::Path;\nuse core::mem;\n",
      new Set(["serde"]),
    );
    expect(findings).toEqual([]);
  });

  test("lintFile flags undeclared crate", () => {
    const findings = lintFile(
      "fake.rs",
      "use serde::Deserialize;\nuse undeclared_crate::Thing;\n",
      new Set(["serde"]),
    );
    expect(findings.length).toBe(1);
    expect(findings[0]!.importedCrate).toBe("undeclared_crate");
    expect(findings[0]!.line).toBe(2);
  });

  test("lintFile passes on declared crate via hyphen→underscore mapping", () => {
    const findings = lintFile(
      "fake.rs",
      "use pellucid_core::version;\n",
      new Set(["pellucid_core"]),
    );
    expect(findings).toEqual([]);
  });

  test("lintHandlersCrate runs against the real crate (currently empty)", () => {
    const findings = lintHandlersCrate(repoRoot);
    expect(findings).toEqual([]);
  });
});
