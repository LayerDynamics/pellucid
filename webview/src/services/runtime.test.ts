import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";

import {
  WEB_API_BASE,
  __pellucidRuntimeInternals,
  detectDesktopRuntime,
  getApiBaseUrl,
  getLocalApiPort,
  getLocalApiToken,
  getLocalApiTokenPrevious,
  installRuntimeFetchPatch,
  installWebApiRedirect,
  isDesktopRuntime,
  resolveLocalApiPort,
  resolveLocalApiToken,
  startSmartPollLoop,
  toApiUrl,
  VisibilityHub,
} from "./runtime";

interface FakeBridge {
  invoke: ReturnType<typeof mock>;
  event?: {
    listen: ReturnType<typeof mock>;
  };
}

function fakeBridge(
  responses: Record<string, unknown> = {},
  listenImpl?: ReturnType<typeof mock>,
): FakeBridge {
  const invoke = mock(async (cmd: string) => {
    if (Object.prototype.hasOwnProperty.call(responses, cmd)) {
      const v = responses[cmd];
      if (v instanceof Error) throw v;
      return v;
    }
    throw new Error(`unknown command: ${cmd}`);
  });
  const bridge: FakeBridge = { invoke };
  if (listenImpl) {
    bridge.event = { listen: listenImpl };
  }
  return bridge;
}

afterEach(() => {
  __pellucidRuntimeInternals.reset();
});

// ---------- detection ----------

describe("isDesktopRuntime", () => {
  test("false when __TAURI__ is missing", () => {
    expect(isDesktopRuntime()).toBe(false);
  });

  test("true when __TAURI__ is installed", () => {
    const cleanup = __pellucidRuntimeInternals.installFakeBridge(
      fakeBridge() as unknown as Parameters<
        typeof __pellucidRuntimeInternals.installFakeBridge
      >[0],
    );
    expect(isDesktopRuntime()).toBe(true);
    cleanup();
  });
});

describe("detectDesktopRuntime", () => {
  test("returns false on web (no __TAURI__)", async () => {
    expect(await detectDesktopRuntime()).toBe(false);
  });

  test("returns true when bridge.invoke resolves", async () => {
    const cleanup = __pellucidRuntimeInternals.installFakeBridge(
      fakeBridge({ get_variant: "base" }) as unknown as Parameters<
        typeof __pellucidRuntimeInternals.installFakeBridge
      >[0],
    );
    expect(await detectDesktopRuntime()).toBe(true);
    cleanup();
  });

  test("returns false when bridge.invoke rejects", async () => {
    const cleanup = __pellucidRuntimeInternals.installFakeBridge(
      fakeBridge({ get_variant: new Error("nope") }) as unknown as Parameters<
        typeof __pellucidRuntimeInternals.installFakeBridge
      >[0],
    );
    expect(await detectDesktopRuntime()).toBe(false);
    cleanup();
  });

  test("caches the result so the second call does not re-invoke", async () => {
    const bridge = fakeBridge({ get_variant: "base" });
    const cleanup = __pellucidRuntimeInternals.installFakeBridge(
      bridge as unknown as Parameters<
        typeof __pellucidRuntimeInternals.installFakeBridge
      >[0],
    );
    await detectDesktopRuntime();
    await detectDesktopRuntime();
    expect(bridge.invoke).toHaveBeenCalledTimes(1);
    cleanup();
  });
});

// ---------- port + token ----------

describe("resolveLocalApiPort", () => {
  test("returns null on web", async () => {
    expect(await resolveLocalApiPort()).toBeNull();
    expect(getLocalApiPort()).toBeNull();
  });

  test("caches the port returned by IPC", async () => {
    const bridge = fakeBridge({ get_local_api_port: 46123 });
    const cleanup = __pellucidRuntimeInternals.installFakeBridge(
      bridge as unknown as Parameters<
        typeof __pellucidRuntimeInternals.installFakeBridge
      >[0],
    );
    expect(await resolveLocalApiPort()).toBe(46123);
    expect(getLocalApiPort()).toBe(46123);
    expect(await resolveLocalApiPort()).toBe(46123);
    expect(bridge.invoke).toHaveBeenCalledTimes(1);
    cleanup();
  });

  test("rejects non-numeric port responses", async () => {
    const cleanup = __pellucidRuntimeInternals.installFakeBridge(
      fakeBridge({ get_local_api_port: "nope" }) as unknown as Parameters<
        typeof __pellucidRuntimeInternals.installFakeBridge
      >[0],
    );
    expect(await resolveLocalApiPort()).toBeNull();
    cleanup();
  });
});

describe("resolveLocalApiToken", () => {
  test("returns null on web", async () => {
    expect(await resolveLocalApiToken()).toBeNull();
  });

  test("caches the bundle from refresh_secrets", async () => {
    const bridge = fakeBridge({
      refresh_secrets: { sidecar_token: "tok-A", sidecar_token_previous: "tok-B" },
    });
    const cleanup = __pellucidRuntimeInternals.installFakeBridge(
      bridge as unknown as Parameters<
        typeof __pellucidRuntimeInternals.installFakeBridge
      >[0],
    );
    expect(await resolveLocalApiToken()).toBe("tok-A");
    expect(getLocalApiToken()).toBe("tok-A");
    expect(getLocalApiTokenPrevious()).toBe("tok-B");
    cleanup();
  });

  test("returns null when IPC rejects", async () => {
    const cleanup = __pellucidRuntimeInternals.installFakeBridge(
      fakeBridge({ refresh_secrets: new Error("boom") }) as unknown as Parameters<
        typeof __pellucidRuntimeInternals.installFakeBridge
      >[0],
    );
    expect(await resolveLocalApiToken()).toBeNull();
    cleanup();
  });
});

// ---------- URL builders ----------

describe("getApiBaseUrl + toApiUrl", () => {
  test("web fallback uses the public WM API base", () => {
    expect(getApiBaseUrl()).toBe(WEB_API_BASE);
    expect(toApiUrl("/api/echo")).toBe(`${WEB_API_BASE}/api/echo`);
  });

  test("normalises a path that omits the leading slash", () => {
    expect(toApiUrl("api/echo")).toBe(`${WEB_API_BASE}/api/echo`);
  });

  test("desktop: 127.0.0.1:<port> after port resolution", async () => {
    const cleanup = __pellucidRuntimeInternals.installFakeBridge(
      fakeBridge({ get_local_api_port: 46123 }) as unknown as Parameters<
        typeof __pellucidRuntimeInternals.installFakeBridge
      >[0],
    );
    await resolveLocalApiPort();
    expect(getApiBaseUrl()).toBe("http://127.0.0.1:46123");
    expect(toApiUrl("/api/echo")).toBe("http://127.0.0.1:46123/api/echo");
    cleanup();
  });

  test("desktop without resolved port falls back to bare 127.0.0.1", () => {
    const cleanup = __pellucidRuntimeInternals.installFakeBridge(
      fakeBridge() as unknown as Parameters<
        typeof __pellucidRuntimeInternals.installFakeBridge
      >[0],
    );
    expect(getApiBaseUrl()).toBe("http://127.0.0.1");
    cleanup();
  });

  test("toApiUrl returns absolute URLs unchanged", () => {
    expect(toApiUrl("https://example.com/api/echo")).toBe(
      "https://example.com/api/echo",
    );
    expect(toApiUrl("http://other.host/x")).toBe("http://other.host/x");
  });
});

// ---------- fetch patch ----------

describe("installRuntimeFetchPatch", () => {
  let originalFetch: typeof fetch;
  let recorded: { url: string; init?: RequestInit }[];

  beforeEach(() => {
    originalFetch = globalThis.fetch;
    recorded = [];
    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.toString()
            : input.url;
      const entry: { url: string; init?: RequestInit } = { url };
      if (init !== undefined) {
        entry.init = init;
      }
      recorded.push(entry);
      return Promise.resolve(
        new Response(JSON.stringify({ ok: true }), {
          status: 200,
          headers: { "content-type": "application/json" },
        }),
      );
    }) as typeof fetch;
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
  });

  test("rewrites /api/* to the cached sidecar URL with bearer", async () => {
    const cleanup = __pellucidRuntimeInternals.installFakeBridge(
      fakeBridge({
        get_local_api_port: 46199,
        refresh_secrets: { sidecar_token: "tok", sidecar_token_previous: null },
      }) as unknown as Parameters<
        typeof __pellucidRuntimeInternals.installFakeBridge
      >[0],
    );
    await resolveLocalApiPort();
    await resolveLocalApiToken();
    const uninstall = installRuntimeFetchPatch();
    await fetch("/api/echo", { method: "POST", body: '{"x":1}' });
    expect(recorded).toHaveLength(1);
    expect(recorded[0]!.url).toBe("http://127.0.0.1:46199/api/echo");
    const headers = new Headers(recorded[0]!.init?.headers);
    expect(headers.get("authorization")).toBe("Bearer tok");
    uninstall();
    cleanup();
  });

  test("passes non-/api URLs through unchanged", async () => {
    __pellucidRuntimeInternals.setCachedPort(46199);
    __pellucidRuntimeInternals.setCachedToken("tok");
    const uninstall = installRuntimeFetchPatch();
    await fetch("https://example.com/x");
    expect(recorded[0]!.url).toBe("https://example.com/x");
    expect(new Headers(recorded[0]!.init?.headers).get("authorization"))
      .toBeNull();
    uninstall();
  });

  test("does not overwrite an Authorization header set by the caller", async () => {
    __pellucidRuntimeInternals.setCachedPort(46199);
    __pellucidRuntimeInternals.setCachedToken("router");
    const uninstall = installRuntimeFetchPatch();
    await fetch("/api/echo", {
      headers: { authorization: "Bearer caller-supplied" },
    });
    expect(new Headers(recorded[0]!.init?.headers).get("authorization")).toBe(
      "Bearer caller-supplied",
    );
    uninstall();
  });

  test("uninstall restores the original fetch", async () => {
    __pellucidRuntimeInternals.setCachedPort(46199);
    __pellucidRuntimeInternals.setCachedToken("tok");
    const beforePatch = globalThis.fetch;
    const uninstall = installRuntimeFetchPatch();
    expect(globalThis.fetch).not.toBe(beforePatch);
    uninstall();
    expect(globalThis.fetch).toBe(beforePatch);
  });

  test("second install is a no-op (idempotent)", () => {
    __pellucidRuntimeInternals.setCachedPort(46199);
    const a = installRuntimeFetchPatch();
    const patched = globalThis.fetch;
    const b = installRuntimeFetchPatch();
    expect(globalThis.fetch).toBe(patched);
    a();
    b(); // second uninstall is a no-op
  });

  test("falls through to original fetch when port cannot be resolved", async () => {
    const uninstall = installRuntimeFetchPatch();
    await fetch("/api/echo");
    expect(recorded[0]!.url).toBe("/api/echo");
    uninstall();
  });
});

describe("installWebApiRedirect", () => {
  let originalFetch: typeof fetch;
  let recorded: string[];

  beforeEach(() => {
    originalFetch = globalThis.fetch;
    recorded = [];
    globalThis.fetch = ((input: RequestInfo | URL) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.toString()
            : input.url;
      recorded.push(url);
      return Promise.resolve(new Response("ok", { status: 200 }));
    }) as typeof fetch;
  });

  afterEach(() => {
    globalThis.fetch = originalFetch;
  });

  test("rewrites /api/* to the public WM base", async () => {
    const uninstall = installWebApiRedirect();
    await fetch("/api/v1/get-flight-status");
    expect(recorded[0]).toBe(`${WEB_API_BASE}/api/v1/get-flight-status`);
    uninstall();
  });

  test("ignores absolute URLs", async () => {
    const uninstall = installWebApiRedirect();
    await fetch("https://example.com/x");
    expect(recorded[0]).toBe("https://example.com/x");
    uninstall();
  });

  test("supports a custom base", async () => {
    const uninstall = installWebApiRedirect({ baseUrl: "https://staging.api" });
    await fetch("/api/echo");
    expect(recorded[0]).toBe("https://staging.api/api/echo");
    uninstall();
  });

  test("uninstall restores fetch", () => {
    const before = globalThis.fetch;
    const uninstall = installWebApiRedirect();
    expect(globalThis.fetch).not.toBe(before);
    uninstall();
    expect(globalThis.fetch).toBe(before);
  });
});

// ---------- VisibilityHub ----------

describe("VisibilityHub", () => {
  test("isVisible reads document.visibilityState", () => {
    const hub = new VisibilityHub();
    expect(hub.isVisible()).toBe(true);
  });

  test("subscribe returns a working unsubscribe", () => {
    const hub = new VisibilityHub();
    const fn = mock(() => undefined);
    const off = hub.subscribe(fn);
    expect(hub.size()).toBe(1);
    off();
    expect(hub.size()).toBe(0);
  });

  test("start + dispatch fires every listener with current visibility", () => {
    const hub = new VisibilityHub();
    hub.start();
    const a = mock((v: boolean) => v);
    const b = mock((v: boolean) => v);
    hub.subscribe(a);
    hub.subscribe(b);
    document.dispatchEvent(new Event("visibilitychange"));
    expect(a).toHaveBeenCalledTimes(1);
    expect(b).toHaveBeenCalledTimes(1);
    hub.stop();
  });

  test("stop clears subscribers + detaches the listener", () => {
    const hub = new VisibilityHub();
    hub.start();
    hub.subscribe(() => undefined);
    hub.stop();
    expect(hub.size()).toBe(0);
    // After stop, dispatch should not throw and should not invoke any
    // listener (which is satisfied by the empty Set).
    document.dispatchEvent(new Event("visibilitychange"));
  });

  test("a throwing subscriber does not silence other subscribers", () => {
    const hub = new VisibilityHub();
    hub.start();
    const ok = mock((v: boolean) => v);
    hub.subscribe(() => {
      throw new Error("boom");
    });
    hub.subscribe(ok);
    document.dispatchEvent(new Event("visibilitychange"));
    expect(ok).toHaveBeenCalledTimes(1);
    hub.stop();
  });
});

// ---------- smart-poll ----------

describe("startSmartPollLoop", () => {
  test("rejects non-positive interval", () => {
    expect(() =>
      startSmartPollLoop({ intervalMs: 0, pollFn: () => undefined }),
    ).toThrow("intervalMs must be > 0");
    expect(() =>
      startSmartPollLoop({ intervalMs: -5, pollFn: () => undefined }),
    ).toThrow();
  });

  test("immediate=true runs pollFn before any interval elapses", async () => {
    const fn = mock(() => undefined);
    const stop = startSmartPollLoop({
      intervalMs: 1_000,
      pollFn: fn,
      immediate: true,
    });
    await new Promise((r) => setTimeout(r, 5));
    expect(fn).toHaveBeenCalledTimes(1);
    stop();
  });

  test("immediate=false skips the first synchronous run", async () => {
    const fn = mock(() => undefined);
    const stop = startSmartPollLoop({
      intervalMs: 50,
      pollFn: fn,
      immediate: false,
    });
    await new Promise((r) => setTimeout(r, 10));
    expect(fn).toHaveBeenCalledTimes(0);
    stop();
  });

  test("stop() cancels any pending tick", async () => {
    const fn = mock(() => undefined);
    const stop = startSmartPollLoop({
      intervalMs: 20,
      pollFn: fn,
      immediate: false,
    });
    stop();
    await new Promise((r) => setTimeout(r, 60));
    expect(fn).toHaveBeenCalledTimes(0);
  });

  test("a rejected pollFn does not stop the loop", async () => {
    let calls = 0;
    const stop = startSmartPollLoop({
      intervalMs: 10,
      immediate: true,
      async pollFn() {
        calls++;
        if (calls === 1) {
          throw new Error("first tick fails");
        }
      },
    });
    await new Promise((r) => setTimeout(r, 60));
    stop();
    expect(calls).toBeGreaterThanOrEqual(2);
  });

  test("uses an injected hub when provided", async () => {
    const hub = new VisibilityHub();
    hub.start();
    const fn = mock(() => undefined);
    const stop = startSmartPollLoop({
      intervalMs: 20,
      pollFn: fn,
      hub,
      immediate: true,
    });
    await new Promise((r) => setTimeout(r, 5));
    expect(hub.size()).toBe(1);
    stop();
    // Stopping the loop must remove the loop's subscriber.
    expect(hub.size()).toBe(0);
    hub.stop();
  });
});
