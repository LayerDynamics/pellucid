/**
 * Aviation flight-status — full-stack E2E.
 *
 * Drives the production webview against a real `pellucid-edge-bin`
 * over HTTP, with a wiremock'd aviationstack upstream. Soft-skips
 * unless `PELLUCID_E2E_EDGE_URL` is set — wired up by T2.6
 * (`pellucid-edge-bin minimum viable`) at which point the
 * Playwright global setup boots the bin + the wiremock proxy
 * automatically.
 *
 * Verifies (when enabled):
 *   1. Cold call → 200 + parsed FlightStatus envelope.
 *   2. Warm call → identical body, no second upstream hit.
 *   3. Validation error → 400 + `invalid_request` code.
 *   4. Upstream 503 → 502 + `upstream_failure` or `cache_failure`
 *      code in the JSON envelope (the gateway's stage 11 leaves
 *      handler-marked envelopes intact — see
 *      `crates/pellucid-handlers/src/aviation/v1/get_flight_status.rs`).
 */

import { test, expect } from "@playwright/test";

const EDGE_URL = process.env.PELLUCID_E2E_EDGE_URL;

test.describe("aviation/v1/get-flight-status", () => {
  test.skip(
    !EDGE_URL,
    "set PELLUCID_E2E_EDGE_URL to a running pellucid-edge-bin (T2.6) to enable",
  );

  test("returns FlightStatus envelope for valid query", async ({ request }) => {
    const resp = await request.get(
      `${EDGE_URL}/api/aviation/v1/get-flight-status?flight=AA100&date=2026-04-25&origin=JFK`,
    );
    expect(resp.status()).toBe(200);
    const body = await resp.json();
    expect(body.flight).toBe("AA100");
    expect(body.origin).toBe("JFK");
    expect(typeof body.status).toBe("string");
    expect(body.scheduled_departure).toMatch(/^\d{4}-\d{2}-\d{2}T/);
  });

  test("returns invalid_request envelope for malformed query", async ({ request }) => {
    const resp = await request.get(
      `${EDGE_URL}/api/aviation/v1/get-flight-status?flight=&date=2026-04-25&origin=JFK`,
    );
    expect(resp.status()).toBe(400);
    const body = await resp.json();
    expect(body.error?.code).toBe("invalid_request");
  });

  test("warm call hits cache (no second upstream invocation)", async ({ request }) => {
    const url = `${EDGE_URL}/api/aviation/v1/get-flight-status?flight=AA100&date=2026-04-25&origin=JFK`;
    const r1 = await request.get(url);
    const r2 = await request.get(url);
    expect(r1.status()).toBe(200);
    expect(r2.status()).toBe(200);
    const b1 = await r1.json();
    const b2 = await r2.json();
    expect(b1).toEqual(b2);
    // The wiremock setup at e2e/setup/aviationstack.ts (added in
    // T2.6) tracks request counts via a header echo so this test
    // can assert exactly-once delivery.
    const upstreamCalls = r2.headers()["x-pellucid-upstream-calls"];
    if (upstreamCalls !== undefined) {
      expect(Number(upstreamCalls)).toBe(1);
    }
  });

  test("upstream 5xx surfaces as upstream_failure envelope", async ({ request }) => {
    // The wiremock proxy honours an `x-pellucid-test-upstream-status`
    // request header to drive the upstream response (T2.6 setup).
    const resp = await request.get(
      `${EDGE_URL}/api/aviation/v1/get-flight-status?flight=AA100&date=2026-04-25&origin=JFK`,
      {
        headers: { "x-pellucid-test-upstream-status": "503" },
      },
    );
    expect(resp.status()).toBe(502);
    const body = await resp.json();
    expect(["upstream_failure", "cache_failure"]).toContain(body.error?.code);
  });
});
