/**
 * Premium gating — full-stack E2E (T2.9 / H4).
 *
 * Samples 5 of the 37 endpoints in
 * `crates/pellucid-auth/src/endpoint_tiers.rs::ENDPOINT_ENTITLEMENTS`,
 * exercises them as Free and Pro tier users, and asserts the
 * gateway gates correctly. Soft-skips unless
 * `PELLUCID_E2E_EDGE_URL` is set — wired up by T2.6
 * (`pellucid-edge-bin minimum viable`) and `PELLUCID_E2E_FREE_TOKEN`
 * / `PELLUCID_E2E_PRO_TOKEN` are issued by the test seed in T2.6's
 * Playwright global-setup.
 *
 * Coverage:
 *   1. Tier-1 path + free-tier user → 200.
 *   2. Tier-1 path + anonymous (no token) → 401.
 *   3. Tier-2 path + free-tier user → 403 + `entitlement_forbidden`.
 *   4. Tier-2 path + pro-tier user → 200.
 *   5. Two more tier-1 paths from different domains → 200 for free.
 *
 * What this proves at the UX level: the H4 fix folded the legacy
 * `PREMIUM_RPC_PATHS` Bearer-`role='pro'` path into
 * `ENDPOINT_ENTITLEMENTS`, so the webview sees a single,
 * deterministic gating contract — no second decision point that
 * could leak premium content via the legacy code path.
 */

import { test, expect } from "@playwright/test";

const EDGE_URL = process.env.PELLUCID_E2E_EDGE_URL;
const FREE_TOKEN = process.env.PELLUCID_E2E_FREE_TOKEN;
const PRO_TOKEN = process.env.PELLUCID_E2E_PRO_TOKEN;

test.describe("auth/premium-gating", () => {
  test.skip(
    !EDGE_URL || !FREE_TOKEN || !PRO_TOKEN,
    "set PELLUCID_E2E_EDGE_URL + PELLUCID_E2E_FREE_TOKEN + PELLUCID_E2E_PRO_TOKEN " +
      "(T2.6 Playwright global-setup wires these once edge-bin is bootable)",
  );

  test("tier-1 path admits free-tier user", async ({ request }) => {
    const resp = await request.get(`${EDGE_URL}/api/aviation/v1/get-notams`, {
      headers: { authorization: `Bearer ${FREE_TOKEN}` },
    });
    expect(resp.status()).toBe(200);
  });

  test("tier-1 path rejects anonymous user with 401", async ({ request }) => {
    const resp = await request.get(`${EDGE_URL}/api/aviation/v1/get-notams`);
    expect(resp.status()).toBe(401);
  });

  test("tier-2 path rejects free-tier user with entitlement_forbidden", async ({
    request,
  }) => {
    const resp = await request.get(
      `${EDGE_URL}/api/market/v1/analyze-stock?ticker=AAPL`,
      { headers: { authorization: `Bearer ${FREE_TOKEN}` } },
    );
    expect(resp.status()).toBe(403);
    expect(resp.headers()["x-pellucid-error"]).toBe("entitlement_forbidden");
  });

  test("tier-2 path admits pro-tier user", async ({ request }) => {
    const resp = await request.get(
      `${EDGE_URL}/api/market/v1/analyze-stock?ticker=AAPL`,
      { headers: { authorization: `Bearer ${PRO_TOKEN}` } },
    );
    expect(resp.status()).toBe(200);
  });

  test.describe("cross-domain tier-1 sample", () => {
    for (const path of [
      "/api/maritime/v1/get-ais-tracks",
      "/api/news/v1/get-breaking",
      "/api/cyber/v1/get-cve-detail",
    ]) {
      test(`${path} admits free-tier user`, async ({ request }) => {
        const resp = await request.get(`${EDGE_URL}${path}`, {
          headers: { authorization: `Bearer ${FREE_TOKEN}` },
        });
        expect(resp.status()).toBe(200);
      });
    }
  });
});
