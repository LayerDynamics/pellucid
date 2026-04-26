import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import {
  Toolbar,
  ToolbarButton,
  ToolbarSeparator,
  ToolbarToggleGroup,
  ToolbarToggleItem,
} from "./Toolbar";

afterEach(() => {
  cleanup();
});

describe("<Toolbar>", () => {
  test("renders buttons + separator", () => {
    render(
      <Toolbar>
        <ToolbarButton>One</ToolbarButton>
        <ToolbarSeparator />
        <ToolbarButton>Two</ToolbarButton>
      </Toolbar>,
    );
    expect(screen.getByText("One")).toBeDefined();
    expect(screen.getByText("Two")).toBeDefined();
    expect(
      document.querySelector('[data-pellucid="toolbar"]'),
    ).not.toBeNull();
  });

  test("toggle group can flip its value", async () => {
    const user = userEvent.setup();
    render(
      <Toolbar>
        <ToolbarToggleGroup type="single" defaultValue="a">
          <ToolbarToggleItem value="a">A</ToolbarToggleItem>
          <ToolbarToggleItem value="b">B</ToolbarToggleItem>
        </ToolbarToggleGroup>
      </Toolbar>,
    );
    const a = screen.getByText("A");
    const b = screen.getByText("B");
    expect(a.getAttribute("data-state")).toBe("on");
    expect(b.getAttribute("data-state")).toBe("off");
    await user.click(b);
    expect(b.getAttribute("data-state")).toBe("on");
  });

  test("button click handler fires", async () => {
    const user = userEvent.setup();
    let clicked = 0;
    render(
      <Toolbar>
        <ToolbarButton onClick={() => clicked++}>Hit</ToolbarButton>
      </Toolbar>,
    );
    await user.click(screen.getByText("Hit"));
    expect(clicked).toBe(1);
  });
});
