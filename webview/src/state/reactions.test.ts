import { afterEach, beforeEach, describe, expect, test } from "bun:test";

import {
  applyVariantReactions,
  installVariantReactions,
  MIGRATION_STORAGE_PREFIX,
  readRecordedMigration,
} from "./reactions";
import { useMapStore } from "./useMapStore";
import { usePanelStore } from "./usePanelStore";
import { useVariantStore, ALL_VARIANTS } from "./useVariantStore";
import { configFor } from "../config/variants";

beforeEach(() => {
  useVariantStore.setState({ variant: "base", switching: false });
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
});

afterEach(() => {
  useVariantStore.setState({ variant: "base", switching: false });
});

describe("applyVariantReactions", () => {
  test("seeds the variant's default map layers after reset", () => {
    useMapStore.getState().setLayers(["leftover-from-prior-variant"]);
    applyVariantReactions("tech");
    const layers = useMapStore.getState().layers;
    expect(layers).toEqual(configFor("tech").defaultMapLayers);
  });

  test("hides panels not in the variant allow-list", () => {
    // Register a panel that is NOT in the tech allow-list.
    usePanelStore.getState().setLayout("aviation/flight-status", { rowSpan: 1 });
    // Also register one that IS in tech's allow-list.
    usePanelStore.getState().setLayout("cyber/incident-feed", { rowSpan: 1 });

    applyVariantReactions("tech");

    expect(usePanelStore.getState().isHidden("aviation/flight-status")).toBe(true);
    expect(usePanelStore.getState().isHidden("cyber/incident-feed")).toBe(false);
  });

  test("re-shows previously hidden panels that are allow-listed under the new variant", () => {
    usePanelStore.getState().setLayout("cyber/incident-feed", { rowSpan: 1 });
    usePanelStore.getState().hide("cyber/incident-feed");
    expect(usePanelStore.getState().isHidden("cyber/incident-feed")).toBe(true);

    applyVariantReactions("tech");

    expect(usePanelStore.getState().isHidden("cyber/incident-feed")).toBe(false);
  });

  test("base variant admits every panel via wildcard", () => {
    usePanelStore.getState().setLayout("aviation/flight-status", { rowSpan: 1 });
    usePanelStore.getState().setLayout("cyber/incident-feed", { rowSpan: 1 });
    usePanelStore.getState().setLayout("market/indices-snapshot", { rowSpan: 1 });

    applyVariantReactions("base");

    expect(usePanelStore.getState().isHidden("aviation/flight-status")).toBe(false);
    expect(usePanelStore.getState().isHidden("cyber/incident-feed")).toBe(false);
    expect(usePanelStore.getState().isHidden("market/indices-snapshot")).toBe(false);
  });

  test("records the variant migration key in localStorage", () => {
    applyVariantReactions("happy");
    expect(readRecordedMigration("happy")).toBe(configFor("happy").migrationKey);
    // Sanity: the key uses the documented prefix.
    expect(
      window.localStorage.getItem(`${MIGRATION_STORAGE_PREFIX}happy`),
    ).toBe(configFor("happy").migrationKey);
  });

  test("clears the switching flag", () => {
    useVariantStore.setState({ switching: true });
    applyVariantReactions("finance");
    expect(useVariantStore.getState().switching).toBe(false);
  });

  test("every variant has a unique migration key", () => {
    const keys = new Set<string>();
    for (const v of ALL_VARIANTS) {
      keys.add(configFor(v).migrationKey);
    }
    expect(keys.size).toBe(ALL_VARIANTS.length);
  });
});

describe("installVariantReactions / subscription wiring", () => {
  test("variant flip via setVariant fires the full reaction set", () => {
    usePanelStore.getState().setLayout("cyber/incident-feed", { rowSpan: 1 });
    usePanelStore.getState().setLayout("aviation/flight-status", { rowSpan: 1 });

    const detach = installVariantReactions();
    useVariantStore.getState().setVariant("tech");

    expect(usePanelStore.getState().isHidden("aviation/flight-status")).toBe(true);
    expect(usePanelStore.getState().isHidden("cyber/incident-feed")).toBe(false);
    expect(useMapStore.getState().layers).toEqual(configFor("tech").defaultMapLayers);
    expect(useVariantStore.getState().switching).toBe(false);

    detach();
  });

  test("returns a callable unsubscribe function", () => {
    const detach = installVariantReactions();
    expect(typeof detach).toBe("function");
    // Calling it must not throw.
    detach();
  });
});

describe("readRecordedMigration", () => {
  test("returns null when no migration has been recorded", () => {
    expect(readRecordedMigration("tech")).toBeNull();
  });

  test("returns the recorded key after applyVariantReactions", () => {
    applyVariantReactions("commodity");
    expect(readRecordedMigration("commodity")).toBe(
      configFor("commodity").migrationKey,
    );
  });
});
