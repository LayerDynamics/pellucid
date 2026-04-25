/**
 * Desktop e2e — verifies that the bundled `pellucid-tauri` binary boots,
 * loads the bundled webview, and shows the Pellucid heading.
 *
 * Run via: `bun run e2e:desktop` (which invokes `wdio run wdio.conf.ts`
 * after producing a release build with `cargo tauri build`). This suite
 * runs on Linux CI under `webkit2gtk-driver`; see wdio.conf.ts for the
 * macOS / Windows notes.
 */

import { browser, $, expect } from "@wdio/globals";

describe("@desktop pellucid-tauri window", () => {
  it("launches the bundled webview and shows the Pellucid heading", async () => {
    await browser.url("/");
    const heading = await $('[data-testid="app-title"]');
    await expect(heading).toBeExisting();
    await expect(heading).toHaveText("Pellucid");
  });

  it("renders the product tagline", async () => {
    const tagline = await $('[data-testid="app-tagline"]');
    await expect(tagline).toBeExisting();
    const text = await tagline.getText();
    expect(text).toContain("situational awareness");
  });
});
