import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import {
  formatPercent,
  formatPrice,
  WatchlistRow,
  type WatchlistEntry,
} from "./WatchlistRow";

afterEach(() => cleanup());

const ENTRY: WatchlistEntry = {
  symbol: "SPY",
  price: 524.12,
  percentChange: 1.24,
  exchange: "PCX",
  currency: "USD",
};

describe("WatchlistRow", () => {
  test("renders symbol + price + percent change", () => {
    render(<WatchlistRow entry={ENTRY} />);
    expect(document.querySelector('[data-field="symbol"]')?.textContent).toBe(
      "SPY",
    );
    expect(document.querySelector('[data-field="price"]')?.textContent).toContain(
      "524.12",
    );
    expect(
      document.querySelector('[data-field="percent-change"]')?.textContent,
    ).toBe("+1.24%");
  });

  test("positive change uses positive tone", () => {
    render(<WatchlistRow entry={ENTRY} />);
    const root = document.querySelector('[data-component="WatchlistRow"]');
    expect(root?.getAttribute("data-tone")).toBe("positive");
  });

  test("negative change uses negative tone", () => {
    render(
      <WatchlistRow
        entry={{ ...ENTRY, percentChange: -0.5 }}
      />,
    );
    const root = document.querySelector('[data-component="WatchlistRow"]');
    expect(root?.getAttribute("data-tone")).toBe("negative");
  });

  test("zero change uses neutral tone", () => {
    render(<WatchlistRow entry={{ ...ENTRY, percentChange: 0 }} />);
    const root = document.querySelector('[data-component="WatchlistRow"]');
    expect(root?.getAttribute("data-tone")).toBe("neutral");
  });

  test("href turns the row into an anchor", () => {
    render(
      <WatchlistRow
        entry={{ ...ENTRY, href: "https://example.com/spy" }}
      />,
    );
    const root = document.querySelector('[data-component="WatchlistRow"]');
    expect(root?.tagName).toBe("A");
    expect(root?.getAttribute("href")).toBe("https://example.com/spy");
  });

  test("no href renders a button", () => {
    render(<WatchlistRow entry={ENTRY} />);
    const root = document.querySelector('[data-component="WatchlistRow"]');
    expect(root?.tagName).toBe("BUTTON");
  });

  test("onSelect fires on click", () => {
    const seen: string[] = [];
    render(
      <WatchlistRow
        entry={ENTRY}
        onSelect={(e) => seen.push(e.symbol)}
      />,
    );
    const root = document.querySelector(
      '[data-component="WatchlistRow"]',
    ) as HTMLButtonElement;
    fireEvent.click(root);
    expect(seen).toEqual(["SPY"]);
  });
});

describe("formatPrice", () => {
  test("fixed-precision + thousands grouping", () => {
    expect(formatPrice(1234.5, 2)).toBe("1,234.50");
  });
  test("0 fractional digits drops the decimal", () => {
    expect(formatPrice(1234, 0)).toBe("1,234");
  });
  test("non-finite returns em-dash", () => {
    expect(formatPrice(Number.NaN, 2)).toBe("—");
  });
});

describe("formatPercent", () => {
  test("positive prepends '+'", () => {
    expect(formatPercent(1.24)).toBe("+1.24%");
  });
  test("negative keeps the sign", () => {
    expect(formatPercent(-0.5)).toBe("-0.50%");
  });
  test("zero renders as '+0.00%'", () => {
    expect(formatPercent(0)).toBe("+0.00%");
  });
  test("non-finite returns em-dash", () => {
    expect(formatPercent(Number.NaN)).toBe("—");
  });
});
