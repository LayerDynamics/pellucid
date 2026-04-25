import { afterEach, describe, expect, test } from "bun:test";

import { useBootStore } from "./useBootStore";

afterEach(() => useBootStore.getState().reset());

describe("useBootStore", () => {
  test("initial phase is idle", () => {
    expect(useBootStore.getState().phase).toBe("idle");
  });

  test("start enters P1 and records timestamps", () => {
    useBootStore.getState().start();
    const s = useBootStore.getState();
    expect(s.phase).toBe("p1-storage-i18n-ml-init");
    expect(s.startedAtMs).toBeGreaterThan(0);
    expect(s.reachedAtMs.idle).toBeGreaterThan(0);
  });

  test("advance moves forward", () => {
    useBootStore.getState().start();
    useBootStore.getState().advance("p2-bootstrap-fast-slow");
    expect(useBootStore.getState().phase).toBe("p2-bootstrap-fast-slow");
  });

  test("advance refuses to go backwards", () => {
    useBootStore.getState().start();
    useBootStore.getState().advance("p3-clerk-auth");
    useBootStore.getState().advance("p1-storage-i18n-ml-init");
    expect(useBootStore.getState().phase).toBe("p3-clerk-auth");
  });

  test("fail moves to errored", () => {
    useBootStore.getState().start();
    useBootStore.getState().fail("oops");
    const s = useBootStore.getState();
    expect(s.phase).toBe("errored");
    expect(s.errorMessage).toBe("oops");
  });

  test("isReady true only at ready phase", () => {
    useBootStore.getState().start();
    expect(useBootStore.getState().isReady()).toBe(false);
    useBootStore.getState().advance("p2-bootstrap-fast-slow");
    useBootStore.getState().advance("p3-clerk-auth");
    useBootStore.getState().advance("p4-panel-layout");
    useBootStore.getState().advance("p5-search-intel-url-state");
    useBootStore.getState().advance("p6-parallel-data-load");
    useBootStore.getState().advance("p7-smart-poll-loop");
    useBootStore.getState().advance("p8-desktop-updater");
    useBootStore.getState().advance("ready");
    expect(useBootStore.getState().isReady()).toBe(true);
  });

  test("hasReached compares phase rank", () => {
    useBootStore.getState().start();
    useBootStore.getState().advance("p4-panel-layout");
    expect(useBootStore.getState().hasReached("p1-storage-i18n-ml-init")).toBe(true);
    expect(useBootStore.getState().hasReached("p4-panel-layout")).toBe(true);
    expect(useBootStore.getState().hasReached("p5-search-intel-url-state")).toBe(false);
  });

  test("reset returns to idle", () => {
    useBootStore.getState().start();
    useBootStore.getState().advance("p3-clerk-auth");
    useBootStore.getState().reset();
    expect(useBootStore.getState().phase).toBe("idle");
    expect(useBootStore.getState().startedAtMs).toBeNull();
  });
});
