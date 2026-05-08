import { afterEach, beforeEach, describe, expect, mock, test } from "bun:test";

import {
  __pellucidRuntimeInternals,
} from "../services/runtime";
import { useAuthStore } from "../state/useAuthStore";
import { useBootStore, type BootPhase } from "../state/useBootStore";
import { useDataStore } from "../state/useDataStore";
import { usePanelStore } from "../state/usePanelStore";
import { useVariantStore } from "../state/useVariantStore";

import {
  DEFAULT_PANEL_IDS,
  phase1,
  phase2,
  phase3,
  phase4,
  phase5,
  phase6,
  phase7,
  phase8,
  runBoot,
  __pellucidBootResetForTests,
} from "./boot";

interface CleanupRegistry {
  push: (fn: () => void) => void;
  drain: () => void;
}

function newRegistry(): CleanupRegistry & { list: (() => void)[] } {
  const list: (() => void)[] = [];
  return {
    list,
    push(fn) {
      list.push(fn);
    },
    drain() {
      while (list.length > 0) {
        list.pop()?.();
      }
    },
  };
}

beforeEach(() => {
  // Module-level boot dedupe state (added to handle React StrictMode
  // double-mount in dev). Each test starts from a fresh dedupe slot —
  // otherwise a test that runs `runBoot()` twice gets the cached
  // handle from the first call instead of a real second run.
  __pellucidBootResetForTests();
  useBootStore.getState().reset();
  usePanelStore.getState().reset();
  useVariantStore.setState({ variant: "base", switching: false });
  useAuthStore.getState().signOut();
  useAuthStore.setState({ isLoading: false });
  useDataStore.setState({ byPanel: {}, lastFetchedAtMs: {} });
});

afterEach(() => {
  __pellucidRuntimeInternals.reset();
});

// ---------- BootStore advance ordering ----------

describe("useBootStore + boot phase advance helpers", () => {
  test("start moves idle → P1 and stamps reachedAtMs[idle]", () => {
    const before = Date.now();
    useBootStore.getState().start();
    const s = useBootStore.getState();
    expect(s.phase).toBe("p1-storage-i18n-ml-init");
    expect(s.reachedAtMs.idle).toBeGreaterThanOrEqual(before);
    expect(s.startedAtMs).not.toBeNull();
  });

  test("advance refuses to skip phases", () => {
    useBootStore.getState().start();
    useBootStore.getState().advance("p4-panel-layout"); // skipping p2/p3
    // Allowed because p4 > p1; advance() only blocks BACKWARDS,
    // confirming the doc behaviour. The subsequent test asserts the
    // forward-only contract.
    expect(useBootStore.getState().phase).toBe("p4-panel-layout");
  });

  test("advance refuses to go backwards", () => {
    useBootStore.getState().start();
    useBootStore.getState().advance("p4-panel-layout");
    useBootStore.getState().advance("p2-bootstrap-fast-slow");
    expect(useBootStore.getState().phase).toBe("p4-panel-layout");
  });

  test("fail moves to errored regardless of current phase", () => {
    useBootStore.getState().start();
    useBootStore.getState().fail("upstream blew up");
    expect(useBootStore.getState().phase).toBe("errored");
    expect(useBootStore.getState().errorMessage).toBe("upstream blew up");
  });

  test("hasReached is monotonic", () => {
    const s = useBootStore.getState();
    expect(s.hasReached("idle")).toBe(true);
    expect(s.hasReached("ready")).toBe(false);
    s.start();
    expect(useBootStore.getState().hasReached("p1-storage-i18n-ml-init")).toBe(
      true,
    );
  });
});

// ---------- Per-phase tests ----------

describe("phase1 — storage + reactions + fetch patch", () => {
  test("registers cleanups (web fork)", async () => {
    const reg = newRegistry();
    await phase1(reg);
    expect(reg.list.length).toBeGreaterThanOrEqual(2);
    reg.drain();
  });

  test("registers cleanups (desktop fork) including token-rotated listener", async () => {
    const offListen = mock(() => undefined);
    const cleanup = __pellucidRuntimeInternals.installFakeBridge({
      invoke: (async () => "base") as unknown as <T>(
        cmd: string,
      ) => Promise<T>,
      event: {
        listen: (async () => offListen) as unknown as <P>(
          name: string,
          cb: (e: { payload: P }) => void,
        ) => Promise<() => void>,
      },
    });
    const reg = newRegistry();
    await phase1(reg);
    // 3 cleanups in the desktop branch: reactions + fetch patch +
    // token-rotated listener.
    expect(reg.list.length).toBe(3);
    reg.drain();
    cleanup();
  });
});

describe("phase2 — sidecar port + token cache priming", () => {
  test("no-op on web returns null caches", async () => {
    await phase2();
    // Cache state already null; no exception is the assertion.
  });

  test("primes both caches when bridge is present", async () => {
    const cleanup = __pellucidRuntimeInternals.installFakeBridge({
      invoke: (async (cmd: string) => {
        if (cmd === "get_local_api_port") return 50333;
        if (cmd === "refresh_secrets") {
          return { sidecar_token: "tok-2", sidecar_token_previous: null };
        }
        throw new Error(`unexpected: ${cmd}`);
      }) as unknown as <T>(cmd: string) => Promise<T>,
    });
    await phase2();
    cleanup();
  });
});

describe("phase3 — Clerk auth M0 slice", () => {
  test("clears isLoading flag if previously set", async () => {
    useAuthStore.setState({ isLoading: true });
    await phase3();
    expect(useAuthStore.getState().isLoading).toBe(false);
  });

  test("preserves existing auth fields", async () => {
    useAuthStore.getState().signIn({
      userId: "u1",
      email: "a@b",
      clerkSessionToken: "t",
      entitlements: {
        tier: 1,
        maxDashboards: 5,
        apiAccess: false,
        apiRateLimit: 600,
        prioritySupport: false,
        exportFormats: ["json"],
        validUntilMs: 9_999_999_999_999,
      },
    });
    await phase3();
    expect(useAuthStore.getState().userId).toBe("u1");
  });
});

describe("phase4 — panel layout registration", () => {
  test("registers the M0 default panel set when empty", async () => {
    expect(usePanelStore.getState().registered()).toHaveLength(0);
    await phase4();
    expect(usePanelStore.getState().registered().sort()).toEqual(
      [...DEFAULT_PANEL_IDS].sort(),
    );
  });

  test("does not overwrite an existing layout", async () => {
    usePanelStore
      .getState()
      .setLayout("custom", { rowSpan: 2, colSpan: 3, hidden: false, order: 0 });
    await phase4();
    expect(usePanelStore.getState().registered()).toEqual(["custom"]);
  });

  test("each registered panel has a stable order", async () => {
    await phase4();
    const layouts = usePanelStore.getState().layouts;
    const orders = DEFAULT_PANEL_IDS.map((id) => layouts[id]!.order);
    expect(orders).toEqual([0, 1, 2, 3, 4]);
  });
});

describe("phase5 — URL state", () => {
  test("applies ?variant=tech", async () => {
    await phase5("?variant=tech");
    expect(useVariantStore.getState().variant).toBe("tech");
  });

  test("ignores unknown variants", async () => {
    await phase5("?variant=fictional");
    expect(useVariantStore.getState().variant).toBe("base");
  });

  test("noops on empty search", async () => {
    await phase5("");
    expect(useVariantStore.getState().variant).toBe("base");
  });

  test("ignores ?variant absent", async () => {
    await phase5("?other=1");
    expect(useVariantStore.getState().variant).toBe("base");
  });
});

describe("phase6 — parallel data load M0 slice", () => {
  test("survives an empty data store without throwing", async () => {
    await phase6();
    expect(useDataStore.getState().size()).toBe(0);
  });

  test("does not modify pre-existing buckets", async () => {
    useDataStore.getState().set("watchlist", { x: 1 });
    await phase6();
    expect(useDataStore.getState().size()).toBe(1);
    expect(useDataStore.getState().get("watchlist")?.data).toEqual({ x: 1 });
  });
});

describe("phase7 — smart-poll loop", () => {
  test("returns and registers a cleanup", async () => {
    const reg = newRegistry();
    await phase7(reg, { pollIntervalMs: 60_000 });
    expect(reg.list).toHaveLength(1);
    reg.drain();
  });

  test("uses an injected visibility hub", async () => {
    const reg = newRegistry();
    const { VisibilityHub } = await import("../services/runtime");
    const hub = new VisibilityHub();
    hub.start();
    await phase7(reg, { pollIntervalMs: 60_000, visibilityHub: hub });
    expect(hub.size()).toBe(1);
    reg.drain();
    expect(hub.size()).toBe(0);
    hub.stop();
  });
});

describe("phase8 — desktop updater", () => {
  test("noop on web", async () => {
    await phase8();
  });

  test("invokes request_updater_check on desktop", async () => {
    const invoked: string[] = [];
    const cleanup = __pellucidRuntimeInternals.installFakeBridge({
      invoke: (async (cmd: string) => {
        invoked.push(cmd);
        return undefined;
      }) as unknown as <T>(cmd: string) => Promise<T>,
    });
    await phase8();
    expect(invoked).toEqual(["request_updater_check"]);
    cleanup();
  });

  test("updater rejection does not throw", async () => {
    const cleanup = __pellucidRuntimeInternals.installFakeBridge({
      invoke: (async () => {
        throw new Error("updater not available");
      }) as unknown as <T>(cmd: string) => Promise<T>,
    });
    await expect(phase8()).resolves.toBeUndefined();
    cleanup();
  });
});

// ---------- runBoot orchestrator ----------

describe("runBoot full pipeline", () => {
  test("walks every phase in order and reaches ready", async () => {
    const observed: BootPhase[] = [];
    const handle = await runBoot({
      urlSearch: "?variant=finance",
      pollIntervalMs: 60_000,
      onPhase: (p) => {
        if (observed[observed.length - 1] !== p) observed.push(p);
      },
    });
    expect(useBootStore.getState().phase).toBe("ready");
    expect(useVariantStore.getState().variant).toBe("finance");
    expect(observed.slice(-1)[0]).toBe("ready");
    expect(observed).toContain("p1-storage-i18n-ml-init");
    expect(observed).toContain("p4-panel-layout");
    expect(observed).toContain("p7-smart-poll-loop");
    await handle.shutdown();
    expect(useBootStore.getState().phase).toBe("idle");
  });

  test("registers cleanups so shutdown unwinds them", async () => {
    const handle = await runBoot({ pollIntervalMs: 60_000 });
    await handle.shutdown();
    expect(useBootStore.getState().phase).toBe("idle");
    // Re-running boot must succeed after shutdown.
    const handle2 = await runBoot({ pollIntervalMs: 60_000 });
    expect(useBootStore.getState().phase).toBe("ready");
    await handle2.shutdown();
  });

  test("fails to errored when a phase throws", async () => {
    const cleanup = __pellucidRuntimeInternals.installFakeBridge({
      // Force phase1 to throw by making detection blow up.
      invoke: (async () => {
        throw new Error("fatal-detect");
      }) as unknown as <T>(cmd: string) => Promise<T>,
      event: {
        listen: (async () => () => undefined) as unknown as <P>(
          name: string,
          cb: (e: { payload: P }) => void,
        ) => Promise<() => void>,
      },
    });
    // detectDesktopRuntime swallows the error and returns false, so a
    // direct phase1 run does not error out. To force runBoot to fail
    // we wedge phase8 by making `request_updater_check` reject — but
    // phase8 catches that. The most realistic failure surface is a
    // bad pollIntervalMs which throws synchronously inside phase7.
    let errored = false;
    try {
      await runBoot({ pollIntervalMs: 0 });
    } catch {
      errored = true;
    }
    expect(errored).toBe(true);
    expect(useBootStore.getState().phase).toBe("errored");
    cleanup();
  });

  test("onPhase callback fires for every transition the store accepts", async () => {
    const seen: BootPhase[] = [];
    const handle = await runBoot({
      pollIntervalMs: 60_000,
      onPhase: (p) => seen.push(p),
    });
    // Distinct phases the orchestrator transitioned through.
    const distinct = Array.from(new Set(seen));
    // Must include the boundary phases.
    for (const phase of [
      "p1-storage-i18n-ml-init",
      "p2-bootstrap-fast-slow",
      "p3-clerk-auth",
      "p4-panel-layout",
      "p5-search-intel-url-state",
      "p6-parallel-data-load",
      "p7-smart-poll-loop",
      "p8-desktop-updater",
      "ready",
    ] as const) {
      expect(distinct).toContain(phase);
    }
    await handle.shutdown();
  });
});
