/**
 * Integration test for cross-store reactions.
 *
 * Mirrors the original WorldMonitor `App.ts:424-449` behaviour: when
 * the user switches variant, the map layer slice resets so cross-
 * variant layers don't bleed across themes.
 */

import { afterEach, describe, expect, test } from "bun:test";

import { configFor } from "../src/config/variants";
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
  test("variant flip resets selection and seeds the new variant's defaults", () => {
    // T2.8 update: the reaction now resets selection + reseeds
    // map layers from the target variant's `defaultMapLayers`,
    // not just to []. Cross-variant layers don't bleed because
    // the slate is wiped before the seed runs.
    const detach = installVariantReactions();
    useMapStore.getState().setLayers(["vessels", "aircraft"]);
    useMapStore.getState().selectFeature("ship-42");
    useVariantStore.getState().setVariant("tech");
    expect(useMapStore.getState().layers).toEqual(configFor("tech").defaultMapLayers);
    expect(useMapStore.getState().selectedFeatureId).toBeNull();
    expect(useVariantStore.getState().switching).toBe(false);
    detach();
  });
});
