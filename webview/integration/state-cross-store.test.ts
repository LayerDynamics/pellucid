/**
 * Integration test for cross-store reactions.
 *
 * Mirrors the original WorldMonitor `App.ts:424-449` behaviour: when
 * the user switches variant, the map layer slice resets so cross-
 * variant layers don't bleed across themes.
 */

import { afterEach, describe, expect, test } from "bun:test";

import { installVariantReactions } from "../src/state/reactions";
import { useMapStore } from "../src/state/useMapStore";
import { useVariantStore } from "../src/state/useVariantStore";

afterEach(() => {
  useVariantStore.setState({ variant: "base", switching: false });
  useMapStore.setState({
    mode: "2d",
    viewport: { longitude: 0, latitude: 20, zoom: 2, bearing: 0, pitch: 0 },
    layers: [],
    selectedFeatureId: null,
  });
});

describe("reactions / variant change", () => {
  test("variant flip clears map layers + selection", () => {
    const detach = installVariantReactions();
    useMapStore.getState().setLayers(["vessels", "aircraft"]);
    useMapStore.getState().selectFeature("ship-42");
    useVariantStore.getState().setVariant("tech");
    expect(useMapStore.getState().layers).toEqual([]);
    expect(useMapStore.getState().selectedFeatureId).toBeNull();
    expect(useVariantStore.getState().switching).toBe(false);
    detach();
  });

  test("setting variant to current value is a no-op", () => {
    const detach = installVariantReactions();
    useMapStore.getState().setLayers(["vessels"]);
    useVariantStore.getState().setVariant("base");
    expect(useMapStore.getState().layers).toEqual(["vessels"]);
    detach();
  });

  test("detach stops further reactions from firing", () => {
    const detach = installVariantReactions();
    detach();
    useMapStore.getState().setLayers(["vessels"]);
    useVariantStore.getState().setVariant("finance");
    expect(useMapStore.getState().layers).toEqual(["vessels"]);
  });
});
