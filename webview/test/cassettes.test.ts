/**
 * Smoke test for Polly.js HTTP cassette infrastructure.
 *
 * Polly is wired here so any subsequent task that needs to record/replay
 * upstream HTTP calls (e.g. T2.5 aviation handler integration with the
 * aviationstack API, T3.x stream clients) can rely on the imports
 * resolving and the constructors being exported correctly.
 *
 * We don't actually record a cassette here — the recording flow needs a
 * real server fixture which lives in the per-task integration tests.
 * This is the universal-mandate "the package is wired" smoke.
 */

import { describe, expect, test } from "bun:test";

describe("Polly.js infrastructure (smoke)", () => {
  test("@pollyjs/core exposes Polly constructor", async () => {
    const mod = await import("@pollyjs/core");
    expect(typeof mod.Polly).toBe("function");
    expect(mod.Polly.VERSION).toBeDefined();
  });

  test("@pollyjs/adapter-fetch exposes default adapter class", async () => {
    const mod = await import("@pollyjs/adapter-fetch");
    expect(typeof mod.default).toBe("function");
    expect(mod.default.id ?? mod.default.name).toBeDefined();
  });

  test("@pollyjs/persister-fs exposes default persister class", async () => {
    const mod = await import("@pollyjs/persister-fs");
    expect(typeof mod.default).toBe("function");
  });
});
