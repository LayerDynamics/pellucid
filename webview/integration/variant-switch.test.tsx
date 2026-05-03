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

// ─── T2.8 — full cross-store reaction wiring ──────────────────
//
// These cases drive the actual T2.8 deliverable: the variant
// switch must reset map layers, enforce the panel allow-list,
// record the migration key, and clear the switching flag. They
// share the data-variant attribute setup above with the T1.6
// cases (intentionally — both reactions run concurrently in
// production).

import { configFor } from "../src/config/variants";
import {
  applyVariantReactions,
  installVariantReactions,
  readRecordedMigration,
} from "../src/state/reactions";
import { useMapStore } from "../src/state/useMapStore";
import { usePanelStore } from "../src/state/usePanelStore";

function seedDemoPanels() {
  for (const id of [
    "aviation/flight-status",
    "cyber/incident-feed",
    "market/indices-snapshot",
    "supply-chain/stress-index",
    "positive-events/feed",
    "news/breaking",
  ]) {
    usePanelStore.getState().setLayout(id, { rowSpan: 1 });
  }
}

describe("T2.8 — cross-store reactions on variant switch", () => {
  beforeEach(() => {
    useMapStore.setState({
      mode: "2d",
      viewport: { longitude: 0, latitude: 20, zoom: 2, bearing: 0, pitch: 0 },
      layers: [],
      selectedFeatureId: null,
    });
    usePanelStore.getState().reset();
    if (typeof window !== "undefined" && window.localStorage) {
      window.localStorage.clear();
    }
    seedDemoPanels();
  });

  test("base → tech: aviation hides, cyber stays, market hides, news stays", () => {
    const detach = installVariantReactions();
    useVariantStore.getState().setVariant("tech");

    expect(usePanelStore.getState().isHidden("aviation/flight-status")).toBe(true);
    expect(usePanelStore.getState().isHidden("cyber/incident-feed")).toBe(false);
    expect(usePanelStore.getState().isHidden("market/indices-snapshot")).toBe(true);
    expect(usePanelStore.getState().isHidden("supply-chain/stress-index")).toBe(true);
    expect(usePanelStore.getState().isHidden("positive-events/feed")).toBe(true);
    expect(usePanelStore.getState().isHidden("news/breaking")).toBe(false);

    expect(useMapStore.getState().layers).toEqual(configFor("tech").defaultMapLayers);
    expect(readRecordedMigration("tech")).toBe(configFor("tech").migrationKey);
    expect(useVariantStore.getState().switching).toBe(false);

    detach();
  });

  test("tech → finance: market un-hides, cyber hides", () => {
    applyVariantReactions("tech");
    const detach = installVariantReactions();
    useVariantStore.getState().setVariant("finance");

    expect(usePanelStore.getState().isHidden("market/indices-snapshot")).toBe(false);
    expect(usePanelStore.getState().isHidden("cyber/incident-feed")).toBe(true);
    expect(usePanelStore.getState().isHidden("supply-chain/stress-index")).toBe(false);
    expect(useMapStore.getState().layers).toEqual(configFor("finance").defaultMapLayers);
    expect(readRecordedMigration("finance")).toBe(configFor("finance").migrationKey);

    detach();
  });

  test("commodity → happy: only positive-events + news survive; HAPPY_PANEL_FIX_KEY recorded", () => {
    applyVariantReactions("commodity");
    const detach = installVariantReactions();
    useVariantStore.getState().setVariant("happy");

    expect(usePanelStore.getState().isHidden("positive-events/feed")).toBe(false);
    expect(usePanelStore.getState().isHidden("news/breaking")).toBe(false);
    expect(usePanelStore.getState().isHidden("aviation/flight-status")).toBe(true);
    expect(usePanelStore.getState().isHidden("cyber/incident-feed")).toBe(true);
    expect(usePanelStore.getState().isHidden("market/indices-snapshot")).toBe(true);
    expect(usePanelStore.getState().isHidden("supply-chain/stress-index")).toBe(true);
    expect(useMapStore.getState().layers).toEqual(configFor("happy").defaultMapLayers);
    expect(readRecordedMigration("happy")).toBe("HAPPY_PANEL_FIX_KEY");

    detach();
  });

  test("happy → base: every panel un-hides via wildcard", () => {
    // Drive the variant store to `happy` so the subsequent
    // `setVariant("base")` is a real change. Going through the
    // store also fires the install subscription, which we want
    // here because the test asserts the full subscribe path.
    const detach = installVariantReactions();
    useVariantStore.getState().setVariant("happy");
    useVariantStore.getState().setVariant("base");

    for (const id of [
      "aviation/flight-status",
      "cyber/incident-feed",
      "market/indices-snapshot",
      "supply-chain/stress-index",
      "positive-events/feed",
      "news/breaking",
    ]) {
      expect(usePanelStore.getState().isHidden(id)).toBe(false);
    }

    detach();
  });
});
