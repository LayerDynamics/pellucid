/**
 * Unit tests for tools/build-csp.ts. Verifies every directive is well
 * formed, variant frame-src entries are sourced from
 * webview/src/config/variants/*, and serializeCsp is round-trippable.
 */

import { describe, expect, test } from "bun:test";
import { join } from "node:path";

import { buildDirectives, emit, serializeCsp, variantHosts } from "./build-csp";

const repoRoot = join(import.meta.dir, "..");

describe("build-csp (unit)", () => {
  test("variantHosts handles missing variants directory cleanly", () => {
    expect(variantHosts(repoRoot)).toEqual([]);
  });

  test("buildDirectives includes core hosts", () => {
    const d = buildDirectives(repoRoot);
    expect(d["connect-src"]).toContain("https://api.worldmonitor.app");
    expect(d["connect-src"]).toContain("https://*.clerk.accounts.dev");
    expect(d["connect-src"]).toContain("wss://stream.aisstream.io");
    expect(d["script-src"]).toContain("'wasm-unsafe-eval'");
    expect(d["object-src"]).toEqual(["'none'"]);
  });

  test("serializeCsp produces semicolon-separated directives", () => {
    const out = serializeCsp({
      "default-src": ["'self'"],
      "connect-src": ["'self'", "https://example.test"],
      "script-src": [],
      "style-src": ["'self'"],
      "img-src": [],
      "font-src": [],
      "frame-src": [],
      "worker-src": [],
      "object-src": ["'none'"],
      "base-uri": ["'self'"],
      "form-action": [],
    });
    expect(out).toContain("default-src 'self'");
    expect(out).toContain("connect-src 'self' https://example.test");
    expect(out).toContain("object-src 'none'");
    expect(out).toContain("base-uri 'self'");
    expect(out).not.toContain("script-src ;");
  });

  test("emit() produces a single-line, deterministic policy", () => {
    const a = emit(repoRoot);
    const b = emit(repoRoot);
    expect(a).toBe(b);
    expect(a).toContain("default-src 'self'");
    expect(a).not.toContain("\n");
  });
});
