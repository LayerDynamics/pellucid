import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import { CpiBreakdown } from "./CpiBreakdown";

afterEach(() => cleanup());

describe("CpiBreakdown", () => {
  test("empty components renders empty state", () => {
    render(<CpiBreakdown components={[]} />);
    expect(
      document
        .querySelector('[data-component="CpiBreakdown"]')
        ?.getAttribute("data-state"),
    ).toBe("empty");
  });

  test("renders one row per component", () => {
    render(
      <CpiBreakdown
        components={[
          { label: "Food", yoyPct: 4.2 },
          { label: "Energy", yoyPct: -2.5 },
          { label: "Shelter", yoyPct: 5.7 },
        ]}
        headline="+3.2% YoY"
      />,
    );
    expect(
      document.querySelectorAll('[data-component="CpiBreakdownRow"]').length,
    ).toBe(3);
    expect(document.querySelector('[data-field="headline"]')?.textContent).toBe(
      "+3.2% YoY",
    );
  });

  test("rows tone-coded by sign", () => {
    render(
      <CpiBreakdown
        components={[
          { label: "Food", yoyPct: 4.2 },
          { label: "Energy", yoyPct: -2.5 },
        ]}
      />,
    );
    const food = document.querySelector(
      '[data-component="CpiBreakdownRow"][data-label="Food"]',
    );
    const energy = document.querySelector(
      '[data-component="CpiBreakdownRow"][data-label="Energy"]',
    );
    expect(food?.getAttribute("data-tone")).toBe("positive");
    expect(energy?.getAttribute("data-tone")).toBe("negative");
  });

  test("bar widths normalised to max abs yoy", () => {
    render(
      <CpiBreakdown
        components={[
          { label: "Food", yoyPct: 5.0 },
          { label: "Energy", yoyPct: -2.5 },
        ]}
      />,
    );
    const bars = document.querySelectorAll(
      '[data-component="CpiBreakdownRow"] [data-field="bar"]',
    ) as NodeListOf<HTMLElement>;
    expect(bars[0]?.style.width).toBe("100.00%");
    expect(bars[1]?.style.width).toBe("50.00%");
  });
});
