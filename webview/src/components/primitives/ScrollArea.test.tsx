import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, screen } from "@testing-library/react";

import { ScrollArea } from "./ScrollArea";

afterEach(() => {
  cleanup();
});

describe("<ScrollArea>", () => {
  test("renders viewport with children", () => {
    render(
      <ScrollArea className="h-32 w-32">
        <div>line 1</div>
        <div>line 2</div>
      </ScrollArea>,
    );
    expect(screen.getByText("line 1")).toBeDefined();
    expect(screen.getByText("line 2")).toBeDefined();
    expect(
      document.querySelector('[data-pellucid="scroll-viewport"]'),
    ).not.toBeNull();
  });

  test("forwards extra className onto the root", () => {
    render(
      <ScrollArea orientation="vertical" className="extra-cls h-32">
        <div>x</div>
      </ScrollArea>,
    );
    const root = document.querySelector('[data-pellucid="scroll-root"]');
    expect(root).not.toBeNull();
    expect(root?.className ?? "").toContain("extra-cls");
    expect(root?.className ?? "").toContain("relative");
  });

  test("viewport carries radix data marker for scroll detection", () => {
    render(
      <ScrollArea orientation="both" className="h-32 w-32">
        <div style={{ width: 1000 }}>wide</div>
      </ScrollArea>,
    );
    const viewport = document.querySelector(
      '[data-pellucid="scroll-viewport"]',
    );
    expect(viewport).not.toBeNull();
    expect(viewport?.getAttribute("data-radix-scroll-area-viewport")).toBe(
      "",
    );
  });
});
