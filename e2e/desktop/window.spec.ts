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

// Under xvfb on the GHA runner, WebKitGTK parses the bundled JS
// + commits the first React paint visibly slower than locally —
// the default `waitforTimeout: 10_000` from wdio.conf.ts trips
// before the boot scaffold's `<header>` is in the DOM. Both
// assertions here gate on React having mounted, so wait
// explicitly for the title element with a 30 s ceiling before
// the existence/text checks run. Past that mount, both elements
// are present synchronously.
const REACT_MOUNT_TIMEOUT_MS = 30_000;

describe("@desktop pellucid-tauri window", () => {
  it("launches the bundled webview and shows the Pellucid heading", async () => {
    await browser.url("/");
    await browser.waitUntil(
      async () => (await $('[data-testid="app-title"]')).isExisting(),
      {
        timeout: REACT_MOUNT_TIMEOUT_MS,
        timeoutMsg: "React never mounted [data-testid=app-title] under wry",
      },
    );
    const heading = await $('[data-testid="app-title"]');
    await expect(heading).toBeExisting();
    await expect(heading).toHaveText("Pellucid");
  });

  it("renders the product tagline", async () => {
    await browser.waitUntil(
      async () => (await $('[data-testid="app-tagline"]')).isExisting(),
      {
        timeout: REACT_MOUNT_TIMEOUT_MS,
        timeoutMsg: "React never mounted [data-testid=app-tagline] under wry",
      },
    );
    const tagline = await $('[data-testid="app-tagline"]');
    await expect(tagline).toBeExisting();
    const text = await tagline.getText();
    expect(text).toContain("situational awareness");
  });
});
