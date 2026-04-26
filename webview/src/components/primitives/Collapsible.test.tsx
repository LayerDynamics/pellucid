import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import {
  Collapsible,
  CollapsibleContent,
  CollapsibleTrigger,
} from "./Collapsible";

afterEach(() => {
  cleanup();
});

describe("<Collapsible>", () => {
  test("hidden by default", () => {
    render(
      <Collapsible>
        <CollapsibleTrigger>Tog</CollapsibleTrigger>
        <CollapsibleContent>Body</CollapsibleContent>
      </Collapsible>,
    );
    const content = document.querySelector(
      '[data-pellucid="collapsible-content"]',
    );
    expect(content?.getAttribute("data-state")).toBe("closed");
  });

  test("clicking trigger toggles state", async () => {
    const user = userEvent.setup();
    render(
      <Collapsible>
        <CollapsibleTrigger>Tog</CollapsibleTrigger>
        <CollapsibleContent>Body</CollapsibleContent>
      </Collapsible>,
    );
    await user.click(screen.getByText("Tog"));
    const content = document.querySelector(
      '[data-pellucid="collapsible-content"]',
    );
    expect(content?.getAttribute("data-state")).toBe("open");
    expect(screen.getByText("Body")).toBeDefined();
  });

  test("defaultOpen renders content immediately", () => {
    render(
      <Collapsible defaultOpen>
        <CollapsibleTrigger>T</CollapsibleTrigger>
        <CollapsibleContent>Visible</CollapsibleContent>
      </Collapsible>,
    );
    expect(screen.getByText("Visible")).toBeDefined();
  });
});
