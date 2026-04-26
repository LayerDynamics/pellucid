/**
 * H2 fix — webview UX during a Convex outage.
 *
 * The H2 contract (SPEC-001 §14.1 + §24): when the entitlement
 * upstream is unreachable, the gateway returns 503 + `Retry-After`
 * with `code = entitlement_upstream_down`. The webview must render
 * an OUTAGE BANNER, never an upgrade-plan prompt.
 *
 * This spec drives a tier-2 endpoint while the upstream is forced
 * offline through the dev server's wiremock proxy. The Rust
 * regression at `crates/pellucid-auth/tests/regression_h2.rs` proves
 * the gateway side end-to-end; this spec proves the webview side
 * branches the right way on the response.
 */

import { expect, test } from "@playwright/test";

const TIER2_PATH = "/api/market/v1/analyze-stock";
const PROXY_TOGGLE = "PELLUCID_E2E_CONVEX_OFFLINE";

test.describe("@auth H2 — entitlement upstream down", () => {
  test("503 response surfaces an outage banner, not an upgrade prompt", async ({
    page,
  }) => {
    // The dev server's wiremock proxy reads this env var to decide
    // whether to return 503 for `/api/internal-entitlements`. The
    // Rust gateway maps that to `entitlement_upstream_down`.
    test.skip(
      process.env[PROXY_TOGGLE] !== "1",
      "Set PELLUCID_E2E_CONVEX_OFFLINE=1 + ensure wiremock proxy is active before running",
    );

    await page.goto("/?variant=base");

    const response = await page.request.get(TIER2_PATH, {
      headers: { authorization: "Bearer dev-test-bearer" },
    });
    expect(response.status()).toBe(503);
    expect(response.headers()["retry-after"]).toBe("30");
    expect(response.headers()["x-pellucid-error"]).toBe(
      "entitlement_upstream_down",
    );

    // The webview must render the outage banner and NOT the upgrade
    // prompt — both surfaces are testid-tagged so the spec can
    // assert directly on which one mounted.
    const banner = page.getByTestId("outage-banner");
    const upgrade = page.getByTestId("upgrade-prompt");
    await expect(banner).toBeVisible({ timeout: 5_000 });
    await expect(upgrade).toHaveCount(0);

    const message = await banner.textContent();
    expect(message ?? "").toMatch(/temporarily/i);
  });

  test("503 response carries the documented JSON envelope shape", async ({
    request,
  }) => {
    test.skip(
      process.env[PROXY_TOGGLE] !== "1",
      "Set PELLUCID_E2E_CONVEX_OFFLINE=1 + ensure wiremock proxy is active before running",
    );

    const response = await request.get(TIER2_PATH, {
      headers: { authorization: "Bearer dev-test-bearer" },
    });
    expect(response.status()).toBe(503);
    const body = await response.json();
    expect(body.code).toBe("entitlement_upstream_down");
    expect(typeof body.message).toBe("string");
  });
});
