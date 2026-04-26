/**
 * Desktop e2e — `toApiUrl('/api/echo')` evaluated inside the bundled
 * Tauri webview must point at the dynamic sidecar port returned by
 * the host's `get_local_api_port` IPC command.
 *
 * Soft-skips when the desktop bridge is not present (web shard).
 */

import { browser, expect } from "@wdio/globals";

interface RuntimeProbeResult {
  base: string;
  url: string;
  desktop: boolean;
  port: number | null;
}

declare global {
  interface Window {
    __pellucidRuntimeProbe?: RuntimeProbeResult;
  }
}

describe("@desktop runtime URL builder", () => {
  it("toApiUrl points at 127.0.0.1:<sidecar port>", async () => {
    await browser.url("/");
    const supported = await browser.execute(async () => {
      const mod = (await import("/src/services/runtime.ts")) as {
        resolveLocalApiPort: () => Promise<number | null>;
        getApiBaseUrl: () => string;
        toApiUrl: (path: string) => string;
        isDesktopRuntime: () => boolean;
      };
      const desktop = mod.isDesktopRuntime();
      if (!desktop) {
        return false;
      }
      const port = await mod.resolveLocalApiPort();
      const base = mod.getApiBaseUrl();
      const url = mod.toApiUrl("/api/echo");
      window.__pellucidRuntimeProbe = { base, url, desktop, port };
      return true;
    });
    if (!supported) {
      return; // soft-skip on web shard
    }
    const probe = await browser.execute(
      () => window.__pellucidRuntimeProbe ?? null,
    );
    expect(probe).not.toBeNull();
    expect(probe!.desktop).toBe(true);
    expect(probe!.port).not.toBeNull();
    expect(probe!.url).toMatch(
      /^http:\/\/127\.0\.0\.1:\d+\/api\/echo$/,
    );
    expect(probe!.url.endsWith(`/api/echo`)).toBe(true);
    expect(probe!.url.startsWith(probe!.base)).toBe(true);
  });
});
