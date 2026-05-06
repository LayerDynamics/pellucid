import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { EconIndicatorTile } from "./EconIndicatorTile";

afterEach(() => cleanup());

describe("EconIndicatorTile", () => {
  test("renders code/label/value/subline", () => {
    render(
      <EconIndicatorTile
        id="unrate"
        code="UNRATE"
        label="Unemployment"
        value="3.8%"
        subline="-0.1pp MoM"
        tone="positive"
      />,
    );
    expect(document.querySelector('[data-field="code"]')?.textContent).toBe("UNRATE");
    expect(document.querySelector('[data-field="label"]')?.textContent).toBe(
      "Unemployment",
    );
    expect(document.querySelector('[data-field="value"]')?.textContent).toBe("3.8%");
    expect(document.querySelector('[data-field="subline"]')?.textContent).toBe(
      "-0.1pp MoM",
    );
    expect(
      document
        .querySelector('[data-component="EconIndicatorTile"]')
        ?.getAttribute("data-tone"),
    ).toBe("positive");
  });

  test("subline omitted when not provided", () => {
    render(
      <EconIndicatorTile id="x" code="X" label="X" value="1" />,
    );
    expect(document.querySelector('[data-field="subline"]')).toBeNull();
  });

  test("onSelect renders as button + fires", () => {
    let called = 0;
    render(
      <EconIndicatorTile
        id="x"
        code="X"
        label="X"
        value="1"
        onSelect={() => {
          called += 1;
        }}
      />,
    );
    const root = document.querySelector(
      '[data-component="EconIndicatorTile"]',
    ) as HTMLButtonElement;
    expect(root.tagName).toBe("BUTTON");
    fireEvent.click(root);
    expect(called).toBe(1);
  });

  test("default tone is neutral", () => {
    render(<EconIndicatorTile id="x" code="X" label="X" value="1" />);
    expect(
      document
        .querySelector('[data-component="EconIndicatorTile"]')
        ?.getAttribute("data-tone"),
    ).toBe("neutral");
  });
});
