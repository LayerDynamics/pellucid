import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, fireEvent, render } from "@testing-library/react";

import { SymbolPicker } from "./SymbolPicker";

afterEach(() => cleanup());

describe("SymbolPicker", () => {
  test("Enter key commits the uppercased + trimmed symbol", () => {
    const calls: string[] = [];
    render(
      <SymbolPicker
        value="SPY"
        onChange={(next) => calls.push(next)}
      />,
    );
    const input = document.querySelector(
      'input[aria-label="Symbol"]',
    ) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "  aapl  " } });
    fireEvent.keyDown(input, { key: "Enter" });
    expect(calls).toEqual(["AAPL"]);
  });

  test("blur commits the typed value", () => {
    const calls: string[] = [];
    render(
      <SymbolPicker value="SPY" onChange={(next) => calls.push(next)} />,
    );
    const input = document.querySelector(
      'input[aria-label="Symbol"]',
    ) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "tsla" } });
    fireEvent.blur(input);
    expect(calls).toEqual(["TSLA"]);
  });

  test("empty commits are dropped", () => {
    const calls: string[] = [];
    render(
      <SymbolPicker value="SPY" onChange={(next) => calls.push(next)} />,
    );
    const input = document.querySelector(
      'input[aria-label="Symbol"]',
    ) as HTMLInputElement;
    fireEvent.change(input, { target: { value: "   " } });
    fireEvent.blur(input);
    expect(calls.length).toBe(0);
  });

  test("preset chips commit immediately on click", () => {
    const calls: string[] = [];
    render(
      <SymbolPicker
        value="SPY"
        presets={["QQQ", "DIA"]}
        onChange={(next) => calls.push(next)}
      />,
    );
    const chip = document.querySelector(
      'button[data-symbol-preset="QQQ"]',
    ) as HTMLButtonElement;
    fireEvent.click(chip);
    expect(calls).toEqual(["QQQ"]);
  });

  test("preset chips reflect the active symbol via aria-pressed", () => {
    render(
      <SymbolPicker
        value="DIA"
        presets={["QQQ", "DIA"]}
        onChange={() => {}}
      />,
    );
    const dia = document.querySelector('button[data-symbol-preset="DIA"]');
    const qqq = document.querySelector('button[data-symbol-preset="QQQ"]');
    expect(dia?.getAttribute("aria-pressed")).toBe("true");
    expect(qqq?.getAttribute("aria-pressed")).toBe("false");
  });

  test("custom label is honoured", () => {
    render(
      <SymbolPicker value="" onChange={() => {}} label="Ticker" />,
    );
    expect(document.querySelector('input[aria-label="Ticker"]')).not.toBeNull();
  });
});
