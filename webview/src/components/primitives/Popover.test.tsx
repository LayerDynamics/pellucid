import { afterEach, describe, expect, test } from "bun:test";
import { cleanup, render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";

import {
  Popover,
  PopoverClose,
  PopoverContent,
  PopoverTrigger,
} from "./Popover";

afterEach(() => {
  cleanup();
});

describe("<Popover>", () => {
  test("trigger opens content", async () => {
    const user = userEvent.setup();
    render(
      <Popover>
        <PopoverTrigger>Open</PopoverTrigger>
        <PopoverContent>Body</PopoverContent>
      </Popover>,
    );
    expect(screen.queryByText("Body")).toBeNull();
    await user.click(screen.getByText("Open"));
    expect(screen.getByText("Body")).toBeDefined();
  });

  test("close button hides content", async () => {
    const user = userEvent.setup();
    render(
      <Popover defaultOpen>
        <PopoverTrigger>O</PopoverTrigger>
        <PopoverContent>
          <PopoverClose>X</PopoverClose>
          Body
        </PopoverContent>
      </Popover>,
    );
    await user.click(screen.getByText("X"));
    expect(screen.queryByText("Body")).toBeNull();
  });

  test("forwards classNames + Pellucid marker", () => {
    render(
      <Popover defaultOpen>
        <PopoverTrigger>O</PopoverTrigger>
        <PopoverContent className="extra-cls">x</PopoverContent>
      </Popover>,
    );
    const node = document.querySelector(
      '[data-pellucid="popover-content"]',
    );
    expect(node?.className ?? "").toContain("extra-cls");
  });
});
