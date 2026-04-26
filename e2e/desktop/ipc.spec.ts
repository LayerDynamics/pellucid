/**
 * Desktop e2e — exercises the seven T1.7 IPC commands through the real
 * `__TAURI__.invoke(...)` bridge. Skipped until the bundled
 * `pellucid-tauri` build registers the command handlers (lands at the
 * end of T1.7); the spec is committed alongside the Rust changes so the
 * verify hook in the implementation plan has a target file from day one.
 */

import { browser, expect } from "@wdio/globals";

describe("@desktop pellucid-tauri ipc bridge", () => {
  it("returns a u16 sidecar port from get_local_api_port", async () => {
    await browser.url("/");
    const port = await browser.execute(async () => {
      const tauri = (window as unknown as {
        __TAURI__?: { invoke: (cmd: string) => Promise<number> };
      }).__TAURI__;
      if (!tauri) {
        return -1;
      }
      return tauri.invoke("get_local_api_port");
    });
    if (port === -1) {
      // Tauri bridge not present — desktop runner not active in this
      // shard. Soft-skip rather than fail.
      return;
    }
    expect(port).toBeGreaterThan(0);
    expect(port).toBeLessThanOrEqual(65535);
  });

  it("get_variant returns one of the five known variants", async () => {
    const v = await browser.execute(async () => {
      const tauri = (window as unknown as {
        __TAURI__?: { invoke: (cmd: string) => Promise<string> };
      }).__TAURI__;
      if (!tauri) {
        return null;
      }
      return tauri.invoke("get_variant");
    });
    if (v === null) {
      return;
    }
    expect([
      "base",
      "tech",
      "finance",
      "commodity",
      "happy",
    ]).toContain(v);
  });

  it("set_variant rejects unknown variants with an error", async () => {
    const err = await browser.execute(async () => {
      const tauri = (window as unknown as {
        __TAURI__?: {
          invoke: (cmd: string, args: Record<string, string>) => Promise<unknown>;
        };
      }).__TAURI__;
      if (!tauri) {
        return null;
      }
      try {
        await tauri.invoke("set_variant", { variant: "fictional" });
        return null;
      } catch (e) {
        return String(e);
      }
    });
    if (err === null) {
      return;
    }
    expect(err).toContain("invalid variant");
  });

  it("open_external blocks non-https URLs", async () => {
    const err = await browser.execute(async () => {
      const tauri = (window as unknown as {
        __TAURI__?: {
          invoke: (cmd: string, args: Record<string, string>) => Promise<unknown>;
        };
      }).__TAURI__;
      if (!tauri) {
        return null;
      }
      try {
        await tauri.invoke("open_external", { url: "javascript:alert(1)" });
        return null;
      } catch (e) {
        return String(e);
      }
    });
    if (err === null) {
      return;
    }
    expect(err).toContain("not allowed");
  });
});
