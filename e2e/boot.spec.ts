/**
 * E2E — verifies the 8-phase boot machine reaches `ready` in the
 * dev-served webview. Asserts every phase indicator transitions to
 * `data-reached="true"` so a regression in any phase surfaces as a
 * pixel-precise DOM diff.
 *
 * Runs against both the web project and (via the desktop variant
 * spec at e2e/desktop/boot.spec.ts) the bundled Tauri build.
 */

import { expect, test } from "@playwright/test";

const REQUIRED_PHASES = [
  "p1-storage-i18n-ml-init",
  "p2-bootstrap-fast-slow",
  "p3-clerk-auth",
  "p4-panel-layout",
  "p5-search-intel-url-state",
  "p6-parallel-data-load",
  "p7-smart-poll-loop",
  "p8-desktop-updater",
  "ready",
] as const;

test.describe("@boot 8-phase orchestrator", () => {
  test("boot reaches ready within 5 seconds", async ({ page }) => {
    await page.goto("/");
    const phaseEl = page.getByTestId("boot-phase");
    await expect(phaseEl).toHaveAttribute("data-phase", "ready", {
      timeout: 5_000,
    });
    await expect(phaseEl).toHaveText("Ready");
  });

  test("every phase trail item is marked reached", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByTestId("boot-phase")).toHaveAttribute(
      "data-phase",
      "ready",
      { timeout: 5_000 },
    );
    for (const phase of REQUIRED_PHASES) {
      const item = page.locator(
        `[data-testid="boot-trail"] li[data-phase="${phase}"]`,
      );
      await expect(item).toHaveAttribute("data-reached", "true");
    }
  });

  test("boot error UI does not appear on a healthy boot", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByTestId("boot-phase")).toHaveAttribute(
      "data-phase",
      "ready",
      { timeout: 5_000 },
    );
    await expect(page.getByTestId("boot-error")).toHaveCount(0);
  });

  test("variant from URL query string is honoured", async ({ page }) => {
    // The web app currently only reads urlSearch when the prop is
    // explicitly threaded. We exercise the production code path —
    // window.location.search — by appending ?variant=tech.
    await page.goto("/?variant=tech");
    await expect(page.getByTestId("boot-phase")).toHaveAttribute(
      "data-phase",
      "ready",
      { timeout: 5_000 },
    );
    await expect(page.getByTestId("active-variant")).toHaveText("tech");
  });
});
