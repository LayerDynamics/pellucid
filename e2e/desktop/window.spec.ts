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

async function dumpDomOnFailure(label: string): Promise<never> {
  // wry doesn't proxy browser console to the wdio log, so when a
  // mount-wait times out we don't otherwise know if the bundle even
  // loaded. Dump the live DOM + a marker for `__pellucidRuntime` so
  // the next CI run answers "did main.tsx ever execute?".
  const snapshot = await browser.execute(() => {
    const runtime = (window as { __pellucidRuntime?: unknown })
      .__pellucidRuntime
      ? "yes"
      : "no";
    return JSON.stringify({
      url: window.location.href,
      runtimeAttached: runtime,
      title: document.title,
      readyState: document.readyState,
      bodyHtml: document.body.innerHTML.slice(0, 2000),
    });
  });
  // eslint-disable-next-line no-console
  console.error(`[window.spec] ${label} — DOM snapshot:`, snapshot);
  throw new Error(`React never mounted ${label} under wry`);
}

describe("@desktop pellucid-tauri window", () => {
  it("launches the bundled webview and shows the Pellucid heading", async () => {
    await browser.url("/");
    try {
      await browser.waitUntil(
        async () => (await $('[data-testid="app-title"]')).isExisting(),
        {
          timeout: REACT_MOUNT_TIMEOUT_MS,
          timeoutMsg: "wait timeout for [data-testid=app-title]",
        },
      );
    } catch {
      await dumpDomOnFailure("[data-testid=app-title]");
    }
    const heading = await $('[data-testid="app-title"]');
    await expect(heading).toBeExisting();
    await expect(heading).toHaveText("Pellucid");
  });

  it("renders the product tagline", async () => {
    try {
      await browser.waitUntil(
        async () => (await $('[data-testid="app-tagline"]')).isExisting(),
        {
          timeout: REACT_MOUNT_TIMEOUT_MS,
          timeoutMsg: "wait timeout for [data-testid=app-tagline]",
        },
      );
    } catch {
      await dumpDomOnFailure("[data-testid=app-tagline]");
    }
    const tagline = await $('[data-testid="app-tagline"]');
    await expect(tagline).toBeExisting();
    const text = await tagline.getText();
    expect(text).toContain("situational awareness");
  });
});
