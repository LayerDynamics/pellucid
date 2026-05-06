import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { CountryEconCard } from "./CountryEconCard";

afterEach(() => cleanup());

describe("CountryEconCard", () => {
  test("renders provided fields, omits missing ones", () => {
    render(
      <CountryEconCard
        summary={{
          country: "United States",
          iso: "US",
          gdpUsdBillion: 27_000,
          gdpYoyPct: 2.5,
          inflationYoyPct: 3.1,
          unemploymentPct: 3.8,
          policyRatePct: 5.25,
        }}
      />,
    );
    expect(document.querySelector('[data-field="country"]')?.textContent).toBe(
      "United States",
    );
    expect(document.querySelector('[data-field="iso"]')?.textContent).toBe("US");
    const gdp = document.querySelector('[data-field="gdp"]');
    expect(gdp?.querySelector("dd")?.textContent).toBe("$27000B");
    expect(
      document.querySelector('[data-field="gdp-yoy"] dd')?.textContent,
    ).toBe("+2.5%");
  });

  test("omits stats not provided", () => {
    render(
      <CountryEconCard summary={{ country: "Country X" }} />,
    );
    expect(document.querySelector('[data-field="gdp"]')).toBeNull();
    expect(document.querySelector('[data-field="iso"]')).toBeNull();
  });

  test("onSelect renders as button + fires with summary", () => {
    const captured: { name: string | null } = { name: null };
    render(
      <CountryEconCard
        summary={{ country: "Iran" }}
        onSelect={(s) => {
          captured.name = s.country;
        }}
      />,
    );
    const root = document.querySelector(
      '[data-component="CountryEconCard"]',
    ) as HTMLButtonElement;
    expect(root.tagName).toBe("BUTTON");
    fireEvent.click(root);
    expect(captured.name).toBe("Iran");
  });

  test("inflation > 4 tone-codes negative", () => {
    render(
      <CountryEconCard summary={{ country: "X", inflationYoyPct: 5.5 }} />,
    );
    const inflation = document.querySelector('[data-field="inflation"] dd');
    expect(inflation?.className).toContain("danger");
  });
});
