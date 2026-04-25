import { afterEach, describe, expect, test } from "bun:test";

import { useDataStore } from "./useDataStore";

afterEach(() => {
  useDataStore.setState({ byPanel: {}, lastFetchedAtMs: {} });
});

describe("useDataStore", () => {
  test("set + get round-trips a typed payload", () => {
    useDataStore.getState().set<{ items: number[] }>("p", { items: [1, 2, 3] });
    const got = useDataStore.getState().get<{ items: number[] }>("p");
    expect(got?.data.items).toEqual([1, 2, 3]);
    expect(got?.isStale).toBe(false);
    expect(got?.fetchedAtMs).toBeGreaterThan(0);
  });

  test("set with isStale flag honors the override", () => {
    useDataStore.getState().set("p", { x: 1 }, { isStale: true });
    expect(useDataStore.getState().get("p")?.isStale).toBe(true);
  });

  test("markStale flips an existing entry", () => {
    useDataStore.getState().set("p", { x: 1 });
    useDataStore.getState().markStale("p");
    expect(useDataStore.getState().get("p")?.isStale).toBe(true);
  });

  test("markStale on missing panel is a no-op", () => {
    useDataStore.getState().markStale("absent");
    expect(useDataStore.getState().get("absent")).toBeUndefined();
  });

  test("clear removes the panel and its timestamp", () => {
    useDataStore.getState().set("p", { x: 1 });
    useDataStore.getState().clear("p");
    expect(useDataStore.getState().get("p")).toBeUndefined();
  });

  test("size reflects byPanel count", () => {
    expect(useDataStore.getState().size()).toBe(0);
    useDataStore.getState().set("a", {});
    useDataStore.getState().set("b", {});
    expect(useDataStore.getState().size()).toBe(2);
  });
});
