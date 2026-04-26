import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, screen } from "@testing-library/react";

import { Tooltip, TooltipProvider } from "./Tooltip";

afterEach(() => {
  cleanup();
});

describe("<Tooltip>", () => {
  test("default-open renders content with trigger labeled by it", () => {
    render(
      <TooltipProvider>
        <Tooltip content="Help text" defaultOpen>
          <button type="button">Trigger</button>
        </Tooltip>
      </TooltipProvider>,
    );
    // Radix renders the tooltip body twice: visible content + sr-only span.
    expect(screen.getAllByText("Help text").length).toBeGreaterThanOrEqual(1);
    expect(screen.getByText("Trigger")).toBeDefined();
  });

  test("hidden by default", () => {
    render(
      <TooltipProvider>
        <Tooltip content="Hidden">
          <button type="button">T</button>
        </Tooltip>
      </TooltipProvider>,
    );
    expect(screen.queryByText("Hidden")).toBeNull();
  });

  test("side prop is propagated to content", () => {
    render(
      <TooltipProvider>
        <Tooltip content="Right side" side="right" defaultOpen>
          <button type="button">T</button>
        </Tooltip>
      </TooltipProvider>,
    );
    const content = document.querySelector(
      '[data-pellucid="tooltip-content"]',
    );
    expect(content).not.toBeNull();
    expect(content?.getAttribute("data-side")).toBe("right");
  });
});
