/**
 * Unit tests for tools/check-csp.ts.
 *
 * Without a wired CSP at any of the three locations (the Rust edge
 * middleware lands at T2.6, the index.html meta lands at T1.6, the
 * tauri.conf.json security.csp at T6.3), every location reports
 * `absent` — and that is currently a non-failing condition. This test
 * proves the harness reads each location and serializes the diff.
 */

import { describe, expect, test } from "bun:test";
import { join } from "node:path";

import {
  diffCsp,
  readEdgeMiddlewareCsp,
  readIndexHtmlCsp,
  readTauriCsp,
  check,
} from "./check-csp";

const repoRoot = join(import.meta.dir, "..");

describe("check-csp (unit)", () => {
  test("readIndexHtmlCsp returns null until index.html ships a meta tag (T1.6)", () => {
    expect(readIndexHtmlCsp(repoRoot)).toBeNull();
  });

  test("readEdgeMiddlewareCsp returns null until pellucid-edge-bin lands (T2.6)", () => {
    expect(readEdgeMiddlewareCsp(repoRoot)).toBeNull();
  });

  test("readTauriCsp returns null while tauri.conf.json sets security.csp = null", () => {
    expect(readTauriCsp(repoRoot)).toBeNull();
  });

  test("diffCsp reports identical strings as ok", () => {
    const result = diffCsp("default-src 'self'", "default-src 'self'");
    expect(result.ok).toBe(true);
  });

  test("diffCsp reports added/missing directives", () => {
    const result = diffCsp(
      "default-src 'self'; connect-src https://a",
      "default-src 'self'; connect-src https://b",
    );
    expect(result.ok).toBe(false);
    expect(result.diff).toContain("missing: connect-src https://a");
    expect(result.diff).toContain("extra:   connect-src https://b");
  });

  test("check() returns absent for every location until they are wired", () => {
    const results = check(repoRoot);
    expect(results.length).toBe(3);
    for (const r of results) {
      expect(["match", "absent"]).toContain(r.status);
    }
  });
});
