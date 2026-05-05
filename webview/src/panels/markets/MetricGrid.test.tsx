import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import { MetricGrid, type MetricTile } from "./MetricGrid";

afterEach(() => cleanup());

const TILES: MetricTile[] = [
  { id: "price", label: "Price", value: "$524.00", subline: "+1.24%", tone: "positive" },
  { id: "vol", label: "Volume", value: "12M", tone: "neutral" },
  { id: "chg", label: "Change", value: "-0.72%", tone: "negative" },
];

describe("MetricGrid", () => {
  test("empty tiles renders an empty-state row", () => {
    const { container } = render(<MetricGrid tiles={[]} />);
    const el = container.querySelector('[data-component="MetricGrid"]');
    expect(el?.getAttribute("data-state")).toBe("empty");
  });

  test("renders one MetricTile per input tile with id + tone", () => {
    render(<MetricGrid tiles={TILES} />);
    const tiles = document.querySelectorAll('[data-component="MetricTile"]');
    expect(tiles.length).toBe(3);
    const tones = Array.from(tiles).map((el) => el.getAttribute("data-tone"));
    expect(tones).toEqual(["positive", "neutral", "negative"]);
  });

  test("subline is omitted when not provided", () => {
    render(
      <MetricGrid
        tiles={[{ id: "x", label: "X", value: "1" }]}
      />,
    );
    expect(document.querySelector('[data-field="subline"]')).toBeNull();
  });

  test("tile-count data attribute matches input length", () => {
    render(<MetricGrid tiles={TILES} />);
    const root = document.querySelector('[data-component="MetricGrid"]');
    expect(root?.getAttribute("data-tile-count")).toBe("3");
  });

  test("clamps columns to [1, 6]", () => {
    render(<MetricGrid tiles={TILES} columns={99} />);
    const root = document.querySelector('[data-component="MetricGrid"]');
    expect(root?.className).toContain("lg:grid-cols-6");
  });
});
