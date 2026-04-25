import { afterEach, describe, expect, test } from "bun:test";

import { useNewsStore, type NewsItem } from "./useNewsStore";

const item = (id: string): NewsItem => ({
  id,
  title: id,
  source: "test",
  publishedAtMs: 1000,
});

afterEach(() => useNewsStore.getState().clear());

describe("useNewsStore", () => {
  test("setFeed copies the outer array so caller mutation does not bleed", () => {
    const items = [item("a"), item("b")];
    useNewsStore.getState().setFeed(items);
    expect(useNewsStore.getState().feed.length).toBe(2);
    // Mutating the caller's source array must not change the stored slice.
    items.push(item("c"));
    expect(useNewsStore.getState().feed.length).toBe(2);
  });

  test("prepend adds at front and dedups by id", () => {
    useNewsStore.getState().setFeed([item("a"), item("b")]);
    useNewsStore.getState().prepend(item("c"));
    expect(useNewsStore.getState().feed.map((x) => x.id)).toEqual(["c", "a", "b"]);
    useNewsStore.getState().prepend(item("a")); // dedup
    expect(useNewsStore.getState().feed.map((x) => x.id)).toEqual(["a", "c", "b"]);
  });

  test("setBreaking + setSignals + setGaps", () => {
    useNewsStore.getState().setBreaking(item("breaking"));
    useNewsStore.getState().setSignals(["s1"]);
    useNewsStore.getState().setGaps(["g1"]);
    const s = useNewsStore.getState();
    expect(s.breaking?.id).toBe("breaking");
    expect(s.signals).toEqual(["s1"]);
    expect(s.gaps).toEqual(["g1"]);
  });

  test("clear empties every slice", () => {
    useNewsStore.getState().setFeed([item("a")]);
    useNewsStore.getState().setBreaking(item("b"));
    useNewsStore.getState().clear();
    expect(useNewsStore.getState().feed).toEqual([]);
    expect(useNewsStore.getState().breaking).toBeNull();
  });

  test("size reflects feed length", () => {
    useNewsStore.getState().setFeed([item("a"), item("b"), item("c")]);
    expect(useNewsStore.getState().size()).toBe(3);
  });
});
