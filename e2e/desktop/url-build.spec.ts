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

interface PellucidRuntime {
  isDesktopRuntime: () => boolean;
  resolveLocalApiPort: () => Promise<number | null>;
  getApiBaseUrl: () => string;
  toApiUrl: (path: string) => string;
}

declare global {
  interface Window {
    __pellucidRuntime?: PellucidRuntime;
    __pellucidRuntimeProbe?: RuntimeProbeResult;
  }
}

describe("@desktop runtime URL builder", () => {
  it("toApiUrl points at 127.0.0.1:<sidecar port>", async () => {
    await browser.url("/");
    // `webview/src/main.tsx` attaches `window.__pellucidRuntime` on
    // every boot so specs can reach the URL-builder helpers without
    // depending on dev-only `/src/...` import paths (the production
    // bundle emits content-hashed chunks, so dynamic-importing a TS
    // source path returns a 404 inside `wry`).
    const supported = await browser.execute(async () => {
      const rt = window.__pellucidRuntime;
      if (!rt) {
        return false;
      }
      const desktop = rt.isDesktopRuntime();
      if (!desktop) {
        return false;
      }
      const port = await rt.resolveLocalApiPort();
      const base = rt.getApiBaseUrl();
      const url = rt.toApiUrl("/api/echo");
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
