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

  test("setVariant flips switching=true", () => {
    useVariantStore.getState().setVariant("tech");
    expect(useVariantStore.getState().switching).toBe(true);
  });

  test("setSwitching toggles the flag", () => {
    useVariantStore.getState().setSwitching(true);
    expect(useVariantStore.getState().switching).toBe(true);
    useVariantStore.getState().setSwitching(false);
    expect(useVariantStore.getState().switching).toBe(false);
  });
});
