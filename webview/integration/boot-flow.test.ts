/**
 * Integration test for `runBoot` — drives the entire 8-phase machine
 * against jsdom + a fake `__TAURI__` bridge. Asserts the terminal
 * state is `ready` and that every cross-store side effect the orchestrator
 * is responsible for has actually fired.
 */

import { afterEach, beforeEach, describe, expect, test } from "bun:test";

import { runBoot } from "../src/app/boot";
import { __pellucidRuntimeInternals } from "../src/services/runtime";
import { useAuthStore } from "../src/state/useAuthStore";
import { useBootStore, type BootPhase } from "../src/state/useBootStore";
import { useDataStore } from "../src/state/useDataStore";
import { usePanelStore } from "../src/state/usePanelStore";
import { useVariantStore } from "../src/state/useVariantStore";

beforeEach(() => {
  useBootStore.getState().reset();
  usePanelStore.getState().reset();
  useDataStore.setState({ byPanel: {}, lastFetchedAtMs: {} });
  useVariantStore.setState({ variant: "base", switching: false });
  useAuthStore.getState().signOut();
});

afterEach(() => {
  __pellucidRuntimeInternals.reset();
});

describe("integration: runBoot end-to-end", () => {
  test("web fork — terminates at ready, populates panel layout, applies URL variant", async () => {
    const transitions: BootPhase[] = [];
    const handle = await runBoot({
      urlSearch: "?variant=tech",
      pollIntervalMs: 60_000,
      onPhase: (p) => transitions.push(p),
    });
    expect(useBootStore.getState().phase).toBe("ready");
    expect(useVariantStore.getState().variant).toBe("tech");
    expect(usePanelStore.getState().registered().length).toBeGreaterThan(0);
    expect(transitions[0]).toBe("p1-storage-i18n-ml-init");
    expect(transitions[transitions.length - 1]).toBe("ready");
    await handle.shutdown();
    expect(useBootStore.getState().phase).toBe("idle");
  });

  test("desktop fork — primes sidecar port + token + invokes updater check", async () => {
    const invoked: { cmd: string; args?: Record<string, unknown> }[] = [];
    const cleanup = __pellucidRuntimeInternals.installFakeBridge({
      invoke: (async (cmd: string, args?: Record<string, unknown>) => {
        const entry: { cmd: string; args?: Record<string, unknown> } = { cmd };
        if (args !== undefined) entry.args = args;
        invoked.push(entry);
        switch (cmd) {
          case "get_variant":
            return "base";
          case "get_local_api_port":
            return 50441;
          case "refresh_secrets":
            return {
              sidecar_token: "host-tok",
              sidecar_token_previous: null,
            };
          case "request_updater_check":
            return undefined;
          case "set_variant":
            return "base";
          default:
            throw new Error(`unexpected cmd: ${cmd}`);
        }
      }) as unknown as <T>(
        cmd: string,
        args?: Record<string, unknown>,
      ) => Promise<T>,
      event: {
        listen: (async () => () => undefined) as unknown as <P>(
          name: string,
          cb: (e: { payload: P }) => void,
        ) => Promise<() => void>,
      },
    });
    const handle = await runBoot({ pollIntervalMs: 60_000 });
    expect(useBootStore.getState().phase).toBe("ready");
    const cmds = invoked.map((e) => e.cmd);
    expect(cmds).toContain("get_local_api_port");
    expect(cmds).toContain("refresh_secrets");
    expect(cmds).toContain("request_updater_check");
    await handle.shutdown();
    cleanup();
  });

  test("invalid pollIntervalMs trips the orchestrator into errored", async () => {
    let caught: unknown = null;
    try {
      await runBoot({ pollIntervalMs: 0 });
    } catch (e) {
      caught = e;
    }
    expect(caught).not.toBeNull();
    expect(useBootStore.getState().phase).toBe("errored");
    expect(useBootStore.getState().errorMessage).toMatch(/intervalMs/);
  });

  test("running boot twice in sequence keeps state consistent", async () => {
    const a = await runBoot({ pollIntervalMs: 60_000 });
    expect(useBootStore.getState().phase).toBe("ready");
    await a.shutdown();
    expect(useBootStore.getState().phase).toBe("idle");

    const b = await runBoot({ pollIntervalMs: 60_000 });
    expect(useBootStore.getState().phase).toBe("ready");
    await b.shutdown();
  });
});
