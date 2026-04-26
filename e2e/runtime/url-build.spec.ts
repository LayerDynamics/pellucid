/**
 * Web e2e — verifies that `toApiUrl('/api/echo')` evaluated inside the
 * dev-served webview points at the public WorldMonitor API base.
 *
 * The dev server returns the SPA shell; we evaluate a tiny script in
 * page context that imports the runtime helpers and invokes
 * `toApiUrl`. The expected result is a URL on the public WM host —
 * never `127.0.0.1` — because no Tauri runtime is present in the web
 * shard.
 */

import { expect, test } from "@playwright/test";

test.describe("@runtime web URL builder", () => {
  test("toApiUrl resolves to the public WorldMonitor host", async ({
    page,
  }) => {
    await page.goto("/");
    const url = await page.evaluate(async () => {
      const mod = await import("/src/services/runtime.ts");
      return mod.toApiUrl("/api/echo");
    });
    expect(url).toMatch(/^https:\/\/api\.worldmonitor\.app\/api\/echo$/);
    expect(url).not.toContain("127.0.0.1");
  });

  test("getApiBaseUrl returns the WM base unchanged", async ({ page }) => {
    await page.goto("/");
    const base = await page.evaluate(async () => {
      const mod = await import("/src/services/runtime.ts");
      return mod.getApiBaseUrl();
    });
    expect(base).toBe("https://api.worldmonitor.app");
  });

  test("isDesktopRuntime is false when __TAURI__ is absent", async ({
    page,
  }) => {
    await page.goto("/");
    const desktop = await page.evaluate(async () => {
      const mod = await import("/src/services/runtime.ts");
      return mod.isDesktopRuntime();
    });
    expect(desktop).toBe(false);
  });
});
