import { afterEach, beforeEach, describe, expect, test } from "bun:test";
import { cleanup, render, screen } from "@testing-library/react";

import { Button } from "../src/components/primitives";
import { useVariantStore, type Variant } from "../src/state/useVariantStore";

const ALL_VARIANTS: Variant[] = [
  "base",
  "tech",
  "finance",
  "commodity",
  "happy",
];

let unsubscribe: (() => void) | null = null;

beforeEach(() => {
  document.documentElement.setAttribute("data-variant", "base");
  // Mirror what `<App>` will install in T1.11 — keep the html attr in sync
  // with the store outside of React so updates are synchronous.
  unsubscribe = useVariantStore.subscribe((s, prev) => {
    if (s.variant !== prev.variant) {
      document.documentElement.setAttribute("data-variant", s.variant);
    }
  });
});

afterEach(() => {
  cleanup();
  unsubscribe?.();
  unsubscribe = null;
  document.documentElement.removeAttribute("data-variant");
  useVariantStore.setState({ variant: "base", switching: false });
});

describe("variant switching", () => {
  test("setVariant updates the html data-variant attribute synchronously", () => {
    for (const v of ALL_VARIANTS) {
      useVariantStore.getState().setVariant(v);
      expect(document.documentElement.getAttribute("data-variant")).toBe(v);
    }
  });

  test("primitives still mount across every variant", () => {
    for (const v of ALL_VARIANTS) {
      cleanup();
      useVariantStore.setState({ variant: v });
      document.documentElement.setAttribute("data-variant", v);
      render(<Button>Hi</Button>);
      const btn = screen.getByRole("button", { name: "Hi" });
      expect(btn.getAttribute("data-variant-button")).toBe("solid");
      expect(document.documentElement.getAttribute("data-variant")).toBe(v);
    }
  });

  test("base is the default value of the store", () => {
    expect(useVariantStore.getState().variant).toBe("base");
  });

  test("setVariant accepts every declared variant", () => {
    for (const v of ALL_VARIANTS) {
      useVariantStore.getState().setVariant(v);
      expect(useVariantStore.getState().variant).toBe(v);
    }
  });
});
