import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import { OhlcChart, buildLayout, type OhlcCandle } from "./OhlcChart";

afterEach(() => cleanup());

const SAMPLE: OhlcCandle[] = [
  { timeMs: 1, open: 100, high: 110, low: 95, close: 105 }, // up
  { timeMs: 2, open: 105, high: 108, low: 95, close: 98 }, // down
  { timeMs: 3, open: 98, high: 100, low: 90, close: 92 }, // down
];

describe("OhlcChart", () => {
  test("empty candles renders an empty placeholder", () => {
    const { container } = render(<OhlcChart candles={[]} />);
    const el = container.querySelector('[data-component="OhlcChart"]');
    expect(el?.getAttribute("data-state")).toBe("empty");
  });

  test("non-empty candles renders one OhlcCandle per input candle", () => {
    render(<OhlcChart candles={SAMPLE} title="SPY" />);
    const candles = document.querySelectorAll(
      '[data-component="OhlcCandle"]',
    );
    expect(candles.length).toBe(3);
  });

  test("up candles colour their body via the success token", () => {
    render(<OhlcChart candles={SAMPLE} />);
    const bodies = document.querySelectorAll('rect[data-field="body"]');
    expect(bodies[0]?.getAttribute("data-up")).toBe("true");
    expect(bodies[1]?.getAttribute("data-up")).toBe("false");
    expect(bodies[2]?.getAttribute("data-up")).toBe("false");
  });

  test("renders a figcaption when a title is provided", () => {
    render(<OhlcChart candles={SAMPLE} title="SPY 1d" />);
    const fig = document.querySelector('[data-component="OhlcChart"] figcaption');
    expect(fig?.textContent).toBe("SPY 1d");
  });

  test("data-candle-count attribute reflects input length", () => {
    render(<OhlcChart candles={SAMPLE} />);
    const root = document.querySelector('[data-component="OhlcChart"]');
    expect(root?.getAttribute("data-candle-count")).toBe("3");
  });
});

describe("buildLayout", () => {
  test("empty input returns zero bars + zero range", () => {
    const layout = buildLayout([], 480, 200);
    expect(layout.bars).toEqual([]);
    expect(layout.priceMin).toBe(0);
    expect(layout.priceMax).toBe(0);
  });

  test("price range covers the global low/high across candles", () => {
    const layout = buildLayout(SAMPLE, 480, 200);
    expect(layout.priceMin).toBe(90);
    expect(layout.priceMax).toBe(110);
  });

  test("up flag matches close >= open", () => {
    const layout = buildLayout(SAMPLE, 480, 200);
    expect(layout.bars[0]?.up).toBe(true);
    expect(layout.bars[1]?.up).toBe(false);
    expect(layout.bars[2]?.up).toBe(false);
  });

  test("body height is non-negative even when open == close", () => {
    const flat: OhlcCandle = {
      timeMs: 1,
      open: 100,
      high: 100,
      low: 100,
      close: 100,
    };
    const layout = buildLayout([flat], 480, 200);
    expect(layout.bars[0]?.bodyHeight).toBeGreaterThanOrEqual(0);
  });
});
