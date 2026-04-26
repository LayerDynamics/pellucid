/**
 * Desktop e2e — `/api/echo` round-trip from the bundled webview.
 *
 * The webview asks the host for the sidecar port and bearer, builds
 * the URL via `toApiUrl('/api/echo')`, posts a payload, and asserts
 * the body comes back unchanged. This is the round-trip that
 * SPEC-001 §28 (M0 exit criteria) calls out by name as proof that
 * Tauri + sidecar + webview all wire up correctly.
 */

import { browser, expect } from "@wdio/globals";

interface EchoBridge {
  invoke: <T>(cmd: string) => Promise<T>;
}

declare global {
  interface Window {
    __pellucidEchoResult?: {
      status: number;
      message: string | null;
      payloadSeen: boolean;
    };
  }
}

describe("@desktop pellucid-sidecar /api/echo", () => {
  it("posts a JSON payload and gets the same fields back", async () => {
    await browser.url("/");
    const supported = await browser.execute(async () => {
      const tauri = (window as unknown as { __TAURI__?: EchoBridge })
        .__TAURI__;
      if (!tauri) {
        return false;
      }
      const port = (await tauri.invoke<number>("get_local_api_port")) ?? 0;
      const token = await tauri.invoke<string | null>(
        "get_local_api_token",
      );
      if (!port || !token) {
        return false;
      }
      const resp = await fetch(`http://127.0.0.1:${port}/api/echo`, {
        method: "POST",
        headers: {
          authorization: `Bearer ${token}`,
          "content-type": "application/json",
        },
        body: JSON.stringify({
          message: "from-webview",
          payload: { v: 42 },
        }),
      });
      const status = resp.status;
      let message: string | null = null;
      let payloadSeen = false;
      if (resp.ok) {
        const body = (await resp.json()) as {
          message?: string | null;
          payload?: { v?: number } | null;
        };
        message = body.message ?? null;
        payloadSeen = body.payload?.v === 42;
      }
      window.__pellucidEchoResult = { status, message, payloadSeen };
      return true;
    });

    if (!supported) {
      // Web shard — the sidecar bridge is not exposed; soft-skip.
      return;
    }
    const result = await browser.execute(
      () => window.__pellucidEchoResult ?? null,
    );
    expect(result).not.toBeNull();
    expect(result!.status).toBe(200);
    expect(result!.message).toBe("from-webview");
    expect(result!.payloadSeen).toBe(true);
  });

  it("rejects requests that omit the bearer", async () => {
    const result = await browser.execute(async () => {
      const tauri = (window as unknown as { __TAURI__?: EchoBridge })
        .__TAURI__;
      if (!tauri) {
        return null;
      }
      const port = (await tauri.invoke<number>("get_local_api_port")) ?? 0;
      if (!port) {
        return null;
      }
      const resp = await fetch(`http://127.0.0.1:${port}/api/echo`);
      return resp.status;
    });
    if (result === null) {
      return;
    }
    expect(result).toBe(401);
  });
});
