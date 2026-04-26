import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render } from "@testing-library/react";

import { Slider } from "./Slider";

afterEach(() => {
  cleanup();
});

describe("<Slider>", () => {
  test("single thumb by default", () => {
    render(<Slider defaultValue={[10]} />);
    const thumbs = document.querySelectorAll('[data-pellucid="slider-thumb"]');
    expect(thumbs.length).toBe(1);
  });

  test("range slider renders requested thumb count", () => {
    render(<Slider defaultValue={[10, 80]} thumbCount={2} />);
    const thumbs = document.querySelectorAll('[data-pellucid="slider-thumb"]');
    expect(thumbs.length).toBe(2);
    expect(thumbs[0]?.getAttribute("data-thumb-index")).toBe("0");
    expect(thumbs[1]?.getAttribute("data-thumb-index")).toBe("1");
  });

  test("track and range nodes are present", () => {
    render(<Slider defaultValue={[5]} />);
    expect(
      document.querySelector('[data-pellucid="slider-track"]'),
    ).not.toBeNull();
    expect(
      document.querySelector('[data-pellucid="slider-range"]'),
    ).not.toBeNull();
  });

  test("disabled prop reaches root", () => {
    render(<Slider defaultValue={[5]} disabled />);
    const root = document.querySelector('[data-pellucid="slider-root"]');
    expect(root?.getAttribute("data-disabled")).not.toBeNull();
  });
});
