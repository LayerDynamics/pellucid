/**
 * Integration test for the runtime helpers — exercises the
 * desktop ↔ web fork without spinning up a real Tauri runtime.
 * Mocks `__TAURI__` for desktop assertions, then clears it for the
 * web assertion, proving `getApiBaseUrl` and `toApiUrl` switch
 * targets purely from the global presence.
 */

import { afterEach, describe, expect, test } from "bun:test";

import {
  WEB_API_BASE,
  __pellucidRuntimeInternals,
  detectDesktopRuntime,
  getApiBaseUrl,
  installRuntimeFetchPatch,
  installWebApiRedirect,
  isDesktopRuntime,
  resolveLocalApiPort,
  resolveLocalApiToken,
  toApiUrl,
} from "../src/services/runtime";

afterEach(() => {
  __pellucidRuntimeInternals.reset();
});

describe("runtime integration — desktop fork", () => {
  test("toApiUrl points at 127.0.0.1:<port> after IPC resolves", async () => {
    const cleanup = __pellucidRuntimeInternals.installFakeBridge({
      // The runtime types `invoke<T>(...)` as generic; we feed it a
      // monomorphic responder and cast at the call site.
      invoke: (async (cmd: string) => {
        if (cmd === "get_variant") return "base";
        if (cmd === "get_local_api_port") return 50111;
        if (cmd === "refresh_secrets") {
          return {
            sidecar_token: "host-bearer",
            sidecar_token_previous: null,
          };
        }
        throw new Error(`unexpected cmd: ${cmd}`);
      }) as unknown as <T>(cmd: string) => Promise<T>,
    });
    expect(isDesktopRuntime()).toBe(true);
    expect(await detectDesktopRuntime()).toBe(true);
    expect(await resolveLocalApiPort()).toBe(50111);
    expect(await resolveLocalApiToken()).toBe("host-bearer");
    expect(getApiBaseUrl()).toBe("http://127.0.0.1:50111");
    expect(toApiUrl("/api/echo")).toBe("http://127.0.0.1:50111/api/echo");
    cleanup();
  });

  test("installRuntimeFetchPatch routes /api/echo through the cached host", async () => {
    const cleanup = __pellucidRuntimeInternals.installFakeBridge({
      invoke: (async (cmd: string) => {
        if (cmd === "get_local_api_port") return 50222;
        if (cmd === "refresh_secrets") {
          return {
            sidecar_token: "tok-int",
            sidecar_token_previous: null,
          };
        }
        throw new Error(`unexpected cmd: ${cmd}`);
      }) as unknown as <T>(cmd: string) => Promise<T>,
    });
    await resolveLocalApiPort();
    await resolveLocalApiToken();
    const original = globalThis.fetch;
    let observed: { url: string; auth: string | null } | null = null;
    globalThis.fetch = ((input: RequestInfo | URL, init?: RequestInit) => {
      const url =
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.toString()
            : input.url;
      observed = {
        url,
        auth: new Headers(init?.headers).get("authorization"),
      };
      return Promise.resolve(new Response("{}", { status: 200 }));
    }) as typeof fetch;

    const uninstall = installRuntimeFetchPatch();
    await fetch("/api/echo");
    uninstall();
    globalThis.fetch = original;
    cleanup();

    expect(observed).not.toBeNull();
    const probe = observed as unknown as {
      url: string;
      auth: string | null;
    };
    expect(probe.url).toBe("http://127.0.0.1:50222/api/echo");
    expect(probe.auth).toBe("Bearer tok-int");
  });
});

describe("runtime integration — web fork", () => {
  test("getApiBaseUrl resolves to the public WM API when __TAURI__ is absent", () => {
    expect(isDesktopRuntime()).toBe(false);
    expect(getApiBaseUrl()).toBe(WEB_API_BASE);
    expect(toApiUrl("/api/echo")).toBe(`${WEB_API_BASE}/api/echo`);
  });

  test("installWebApiRedirect rewrites /api to the WM base", async () => {
    const original = globalThis.fetch;
    const captured: string[] = [];
    globalThis.fetch = ((input: RequestInfo | URL) => {
      captured.push(
        typeof input === "string"
          ? input
          : input instanceof URL
            ? input.toString()
            : input.url,
      );
      return Promise.resolve(new Response("{}", { status: 200 }));
    }) as typeof fetch;
    const uninstall = installWebApiRedirect();
    await fetch("/api/v1/health");
    uninstall();
    globalThis.fetch = original;
    expect(captured).toContain(`${WEB_API_BASE}/api/v1/health`);
  });
});

describe("runtime integration — fork switch is observable", () => {
  test("detection cache flips when the bridge is installed and removed", async () => {
    expect(await detectDesktopRuntime()).toBe(false);
    __pellucidRuntimeInternals.reset();
    const cleanup = __pellucidRuntimeInternals.installFakeBridge({
      invoke: (async () => "base") as unknown as <T>(
        cmd: string,
      ) => Promise<T>,
    });
    expect(await detectDesktopRuntime()).toBe(true);
    cleanup();
    __pellucidRuntimeInternals.reset();
    expect(await detectDesktopRuntime()).toBe(false);
  });
});
