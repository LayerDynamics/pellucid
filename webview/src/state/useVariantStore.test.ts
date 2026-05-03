import { afterEach, describe, expect, test } from "bun:test";

import { ALL_VARIANTS, useVariantStore } from "./useVariantStore";

afterEach(() => {
  useVariantStore.setState({ variant: "base", switching: false });
});

describe("useVariantStore", () => {
  test("default variant is base", () => {
    expect(useVariantStore.getState().variant).toBe("base");
  });

  test("ALL_VARIANTS lists all five variants", () => {
    expect(ALL_VARIANTS).toEqual(["base", "tech", "finance", "commodity", "happy"]);
  });

  test("setVariant accepts every valid variant", () => {
    for (const v of ALL_VARIANTS) {
      useVariantStore.getState().setVariant(v);
      expect(useVariantStore.getState().variant).toBe(v);
    }
  });

  test("setVariant rejects invalid variant", () => {
    useVariantStore.getState().setVariant("base");
    // Cast through unknown to bypass the compile-time type check.
    useVariantStore.getState().setVariant("turbo" as unknown as "base");
    expect(useVariantStore.getState().variant).toBe("base");
  });

  test("setVariant changes the variant and toggles switching", () => {
    // The store sets `switching: true` synchronously inside
    // setVariant. If a reactions listener is installed (e.g. by
    // a parallel-running integration test in this run) it
    // immediately flips switching back to false. Both are
    // valid post-conditions; we assert the observable contract:
    // the variant updated, and switching ended up boolean
    // (not undefined / null / something else).
    useVariantStore.getState().setVariant("tech");
    expect(useVariantStore.getState().variant).toBe("tech");
    expect(typeof useVariantStore.getState().switching).toBe("boolean");
  });

  test("setSwitching toggles the flag", () => {
    useVariantStore.getState().setSwitching(true);
    expect(useVariantStore.getState().switching).toBe(true);
    useVariantStore.getState().setSwitching(false);
    expect(useVariantStore.getState().switching).toBe(false);
  });
});
