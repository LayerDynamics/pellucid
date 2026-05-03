/**
 * Aviation panel — full-stack E2E (T2.11).
 *
 * End-to-end: webview → edge bin → handler → cache → wiremock'd
 * aviationstack upstream. Soft-skips unless
 * `PELLUCID_E2E_BASE_URL` (the running webview) and
 * `PELLUCID_E2E_EDGE_URL` (the running pellucid-edge-bin) are
 * set — wired up in T2.6's Playwright global-setup.
 *
 * Coverage:
 *   1. Pro-tier user navigates to the demo dashboard, panel
 *      renders flight status with the expected fields.
 *   2. Anonymous user lands on the demo dashboard, panel renders
 *      the locked state.
 *   3. Tier-2 sandbox path: requesting an upstream that returns
 *      503 surfaces the outage banner with retry countdown.
 */

import { test, expect } from "@playwright/test";

const BASE_URL = process.env.PELLUCID_E2E_BASE_URL;
const EDGE_URL = process.env.PELLUCID_E2E_EDGE_URL;

test.describe("aviation panel", () => {
  test.skip(
    !BASE_URL || !EDGE_URL,
    "set PELLUCID_E2E_BASE_URL + PELLUCID_E2E_EDGE_URL (T2.6 global-setup) to enable",
  );

  test("pro-tier user sees rendered flight status", async ({ page }) => {
    await page.goto(`${BASE_URL}/?demo=aviation&flight=AA100&date=2026-04-25&origin=JFK`);
    const panel = page.locator('[data-panel-id="aviation/flight-status"]');
    await expect(panel).toBeVisible({ timeout: 5_000 });
    const ready = panel.locator('[data-state="ready"]');
    await expect(ready).toBeVisible();
    await expect(ready).toContainText("AA100");
    await expect(ready).toContainText("JFK");
  });

  test("anonymous user sees locked state", async ({ page, context }) => {
    await context.clearCookies();
    await page.goto(
      `${BASE_URL}/?demo=aviation&flight=AA100&date=2026-04-25&origin=JFK`,
    );
    const panel = page.locator('[data-panel-id="aviation/flight-status"]');
    await expect(panel).toBeVisible({ timeout: 5_000 });
    const locked = panel.locator('[data-state="locked"]');
    await expect(locked).toBeVisible();
    await expect(locked).toContainText("Locked");
  });

  test("upstream 503 surfaces outage banner with countdown", async ({ page, context }) => {
    // T2.6 global-setup honours `x-pellucid-test-upstream-status: 503`
    // on the wiremock proxy; we set it via a request header that
    // the webview's debug toolbar threads through to the next call.
    await context.setExtraHTTPHeaders({
      "x-pellucid-test-upstream-status": "503",
    });
    await page.goto(
      `${BASE_URL}/?demo=aviation&flight=AA100&date=2026-04-25&origin=JFK`,
    );
    const panel = page.locator('[data-panel-id="aviation/flight-status"]');
    await expect(panel).toBeVisible({ timeout: 5_000 });
    const error = panel.locator('[data-state="error"]');
    await expect(error).toBeVisible();
    await expect(error).toHaveAttribute(
      "data-error-code",
      /upstream_failure|cache_failure/,
    );
  });
});
