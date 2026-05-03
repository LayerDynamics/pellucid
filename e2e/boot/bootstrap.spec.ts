/**
 * Bootstrap two-tier hydration — full-stack E2E (T2.7).
 *
 * Drives a real `pellucid-edge-bin` over HTTP, with a populated
 * cache. Soft-skips unless `PELLUCID_E2E_EDGE_URL` is set —
 * wired up by T2.6 (`pellucid-edge-bin minimum viable`).
 *
 * Coverage:
 *   1. `?tier=fast` returns 200 within 3 s budget.
 *   2. `?tier=slow` returns 200 within 5 s budget.
 *   3. Both tier responses carry a sensible `data` map.
 *   4. With cache emptied, fast tier returns 503 + `Retry-After: 30`
 *      (M4 outage banner path).
 */

import { test, expect } from "@playwright/test";

const EDGE_URL = process.env.PELLUCID_E2E_EDGE_URL;

test.describe("bootstrap two-tier hydration", () => {
  test.skip(
    !EDGE_URL,
    "set PELLUCID_E2E_EDGE_URL to a running pellucid-edge-bin (T2.6) to enable",
  );

  test("fast tier returns 200 within 3 s budget", async ({ request }) => {
    const t0 = Date.now();
    const resp = await request.get(`${EDGE_URL}/api/bootstrap/v1/get?tier=fast`);
    const elapsed = Date.now() - t0;
    expect(resp.status()).toBe(200);
    expect(elapsed).toBeLessThanOrEqual(3000);
    const body = await resp.json();
    expect(typeof body.data).toBe("object");
    expect(Array.isArray(body.missing)).toBe(true);
  });

  test("slow tier returns 200 within 5 s budget", async ({ request }) => {
    const t0 = Date.now();
    const resp = await request.get(`${EDGE_URL}/api/bootstrap/v1/get?tier=slow`);
    const elapsed = Date.now() - t0;
    expect(resp.status()).toBe(200);
    expect(elapsed).toBeLessThanOrEqual(5000);
    const body = await resp.json();
    expect(typeof body.data).toBe("object");
  });

  test("M4 outage path: empty cache yields 503 + Retry-After 30", async ({ request }) => {
    // T2.6 global-setup honours `x-pellucid-test-clear-cache: 1`
    // to reset the in-memory KV before the request, so this test
    // can drive the M4 path deterministically without polluting
    // shared state across tests.
    const resp = await request.get(
      `${EDGE_URL}/api/bootstrap/v1/get?tier=fast`,
      { headers: { "x-pellucid-test-clear-cache": "1" } },
    );
    expect(resp.status()).toBe(503);
    expect(resp.headers()["retry-after"]).toBe("30");
    const body = await resp.json();
    expect(body.error?.code).toBe("bootstrap_upstream_empty");
    expect(body.error?.retry_after_secs).toBe(30);
  });

  test("invalid tier param → 400", async ({ request }) => {
    const resp = await request.get(`${EDGE_URL}/api/bootstrap/v1/get?tier=BOGUS`);
    expect(resp.status()).toBe(400);
    const body = await resp.json();
    expect(body.error?.code).toBe("invalid_request");
  });
});
