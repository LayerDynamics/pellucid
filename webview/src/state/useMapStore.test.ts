import { afterEach, describe, expect, test } from "bun:test";

import { useMapStore } from "./useMapStore";

afterEach(() => {
  useMapStore.setState({
    mode: "2d",
    viewport: { longitude: 0, latitude: 20, zoom: 2, bearing: 0, pitch: 0 },
    layers: [],
    selectedFeatureId: null,
  });
});

describe("useMapStore", () => {
  test("default mode is 2d", () => {
    expect(useMapStore.getState().mode).toBe("2d");
  });

  test("setMode flips between 2d and 3d", () => {
    useMapStore.getState().setMode("3d");
    expect(useMapStore.getState().mode).toBe("3d");
    useMapStore.getState().setMode("2d");
    expect(useMapStore.getState().mode).toBe("2d");
  });

  test("setViewport merges fields", () => {
    useMapStore.getState().setViewport({ zoom: 8, bearing: 45 });
    const vp = useMapStore.getState().viewport;
    expect(vp.zoom).toBe(8);
    expect(vp.bearing).toBe(45);
    expect(vp.latitude).toBe(20);
  });

  test("toggleLayer adds and removes", () => {
    useMapStore.getState().toggleLayer("vessels");
    expect(useMapStore.getState().layers).toEqual(["vessels"]);
    useMapStore.getState().toggleLayer("vessels");
    expect(useMapStore.getState().layers).toEqual([]);
  });

  test("setLayers replaces the slice", () => {
    useMapStore.getState().setLayers(["a", "b"]);
    expect(useMapStore.getState().layers).toEqual(["a", "b"]);
  });

  test("selectFeature + null clear", () => {
    useMapStore.getState().selectFeature("ship-42");
    expect(useMapStore.getState().selectedFeatureId).toBe("ship-42");
    useMapStore.getState().selectFeature(null);
    expect(useMapStore.getState().selectedFeatureId).toBeNull();
  });

  test("resetLayers clears layers + selection", () => {
    useMapStore.getState().setLayers(["a"]);
    useMapStore.getState().selectFeature("x");
    useMapStore.getState().resetLayers();
    expect(useMapStore.getState().layers).toEqual([]);
    expect(useMapStore.getState().selectedFeatureId).toBeNull();
  });
});
