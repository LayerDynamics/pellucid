import { expect, test } from "@playwright/test";

test.describe("@smoke webview boot", () => {
  test("loads the dev server and renders the Pellucid heading", async ({ page }) => {
    await page.goto("/");
    await expect(page.getByTestId("app-title")).toHaveText("Pellucid");
    await expect(page.getByTestId("app-tagline")).toContainText("situational awareness");
  });

  test("body has no console errors at idle", async ({ page }) => {
    const errors: string[] = [];
    page.on("pageerror", (err) => errors.push(err.message));
    page.on("console", (msg) => {
      if (msg.type() === "error") errors.push(msg.text());
    });

    await page.goto("/");
    await expect(page.getByTestId("app-root")).toBeVisible();
    expect(errors).toEqual([]);
  });
});
