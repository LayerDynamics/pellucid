/**
 * Visual regression baseline — captures a golden screenshot of the boot
 * surface. Subsequent tasks add per-variant + per-panel goldens; this is
 * the universal-mandate seed so the visual project actually has at least
 * one passing test from T0.8 onward.
 */

import { expect, test } from "@playwright/test";

test.describe("@visual webview baseline", () => {
  test("boot screen matches golden", async ({ page }) => {
    await page.goto("/");
    await page.waitForSelector('[data-testid="app-title"]');
    await expect(page).toHaveScreenshot("boot.png", {
      fullPage: false,
      animations: "disabled",
    });
  });
});
